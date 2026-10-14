#!/usr/bin/env python3
"""Check source layout and direct dependencies at the server's module boundaries.

This is a source hygiene check, not a complete Rust name resolver. Cargo remains
responsible for privacy/type checking; aliases and macro-generated dependencies
still require review.
"""
from pathlib import Path
import re


ROOT = Path(__file__).resolve().parents[1]
FORBIDDEN = {
    "contracts": {"app", "backend", "game", "content", "infra", "transport", "admin"},
    "backend": {"app", "transport", "admin"},
    "game": {"app", "backend", "transport", "admin"},
    "content": {"app", "backend", "game", "transport", "admin"},
}


def split_items(text):
    depth = 0
    start = 0
    for index, char in enumerate(text):
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
        elif char == "," and depth == 0:
            yield text[start:index].strip()
            start = index + 1
    yield text[start:].strip()


def imports(text, prefix=""):
    """Expand Rust use trees, including grouped self/super imports."""
    if "{" in text:
        before, rest = text.split("{", 1)
        for child in split_items(rest.rsplit("}", 1)[0]):
            if child:
                yield from imports(child, prefix + before.strip())
    else:
        yield prefix + re.split(r"\s+as\s+", text.strip())[0]


def mask_non_code(text):
    """Keep offsets while masking Rust literals and nested comments."""
    result = list(text)
    index = 0
    raw_literal = re.compile(r'(?:br|cr|r)(\#*)"')
    char_literal = re.compile(r"'(?:\\(?:u\{[^}]*\}|x[\da-fA-F]{2}|.)|[^'\\\n])'")
    while index < len(text):
        start = index
        raw = raw_literal.match(text, index)
        if text.startswith("//", index):
            end = text.find("\n", index)
            index = len(text) if end < 0 else end
        elif text.startswith("/*", index):
            index += 2
            depth = 1
            while index < len(text) and depth:
                if text.startswith("/*", index):
                    depth += 1
                    index += 2
                elif text.startswith("*/", index):
                    depth -= 1
                    index += 2
                else:
                    index += 1
        elif raw:
            delimiter = '"' + raw.group(1)
            end = text.find(delimiter, raw.end())
            index = len(text) if end < 0 else end + len(delimiter)
        elif text[index] == '"':
            index += 1
            while index < len(text):
                if text[index] == "\\":
                    index += 2
                elif text[index] == '"':
                    index += 1
                    break
                else:
                    index += 1
        elif text[index] == "'" and (char := char_literal.match(text, index)):
            index = char.end()
        else:
            index += 1
            continue
        for position in range(start, min(index, len(text))):
            if result[position] != "\n":
                result[position] = " "
    return "".join(result)


def production_source(text):
    code = mask_non_code(text)
    result = list(code)
    # Remove only test-gated items, not the production items that follow them.
    pattern = r"#\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*(?:#\[[^]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?(?:mod|use|(?:async\s+)?fn)\b[^;{]*[;{]"
    for match in re.finditer(pattern, code):
        end = match.end()
        if code[end - 1] == "{":
            depth = 1
            while end < len(code) and depth:
                depth += (code[end] == "{") - (code[end] == "}")
                end += 1
        result[match.start():end] = " " * (end - match.start())
    return "".join(result)


def violations(root):
    problems = []
    for crate in ("server", "protocol"):
        source = root / "crates" / crate / "src"
        allowed = {"lib.rs", "main.rs"} if crate == "server" else {"lib.rs", "tls.rs"}
        for path in sorted(source.glob("*.rs")):
            if path.name not in allowed:
                problems.append(f"{path.relative_to(root)}: place source in its owning module")
        for path in sorted(source.rglob("*.rs")):
            text = path.read_text()
            name = str(path.relative_to(root))
            code = mask_non_code(text)
            if re.search(r"#\[path\s*=", code):
                problems.append(f"{name}: use standard directory modules instead of #[path]")
            if any(code[m.start():m.start() + 7] == "include" for m in re.finditer(r'include_(?:str|bytes)!\(\s*"\.\.?/', text)):
                problems.append(f"{name}: anchor embedded resources to CARGO_MANIFEST_DIR")
            if crate != "server" or path.stem == "tests" or path.stem.endswith("_tests"):
                continue
            relative = path.relative_to(source)
            forbidden = FORBIDDEN.get(relative.parts[0], set())
            if not forbidden:
                continue
            code = production_source(text)
            dependencies = set(re.findall(r"\bcrate\s*::\s*(\w+)", code))
            module = list(relative.with_suffix("").parts)
            if module[-1] == "mod":
                module.pop()
            for expression in re.findall(r"\buse\s+([^;]+);", code):
                for imported in imports(expression):
                    parts = re.split(r"\s*::\s*", imported)
                    if parts[0] == "crate":
                        dependencies.update(parts[1:2])
                    elif parts[0] in {"self", "super"}:
                        base = module.copy()
                        while parts and parts[0] in {"self", "super"}:
                            if parts.pop(0) == "super" and base:
                                base.pop()
                        dependencies.update((base + parts)[:1])
            for expression in re.findall(r"\b(?:super|self)(?:\s*::\s*\w+)+", code):
                parts = re.split(r"\s*::\s*", expression)
                base = module.copy()
                while parts and parts[0] in {"self", "super"}:
                    if parts.pop(0) == "super" and base:
                        base.pop()
                dependencies.update((base + parts)[:1])
            for dependency in sorted(dependencies & forbidden):
                problems.append(f"{name}: {relative.parts[0]} must not depend on {dependency}")
    return problems


if __name__ == "__main__":
    errors = violations(ROOT)
    if errors:
        raise SystemExit("\n".join(errors))
    print("Source layout and module dependency checks passed.")
