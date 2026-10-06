#!/usr/bin/env python3
"""Disassemble the fixed local samples without executing them.

Output is JSON. Absolute-address matches are candidates, not verified code xrefs.
Native disassembly requires a caller-selected instruction boundary (RVA).
"""
import argparse
import json
import struct
from pathlib import Path

import dnfile
import pefile
from capstone import Cs, CS_ARCH_X86, CS_MODE_32
from dncil.cil.body import CilMethodBody
from dncil.cil.body.reader import CilMethodBodyReaderBytes
from dncil.clr.token import StringToken, Token

from clientlib import DEFAULT_WORKSPACE, checked_sample, sha256


def resolve_token(pe, token):
    if isinstance(token, StringToken):
        return {"string": pe.net.user_strings.get(token.rid).value}
    if isinstance(token, Token):
        table = pe.net.mdtables.tables.get(token.table)
        row = table.rows[token.rid - 1] if table else None
        result = {"token": hex(token.value)}
        if row is not None:
            if hasattr(row, "Name"):
                result["name"] = str(row.Name)
            if hasattr(row, "TypeName"):
                result["type"] = str(row.TypeNamespace) + "." + str(row.TypeName)
            if hasattr(row, "Class"):
                owner = row.Class.row
                if hasattr(owner, "TypeName"):
                    result["owner"] = str(owner.TypeNamespace) + "." + str(owner.TypeName)
        return result
    if isinstance(token, (int, float, str, type(None))):
        return token
    if isinstance(token, list):
        return [resolve_token(pe, t) for t in token]
    return str(token)


def launcher(path):
    pe = dnfile.dnPE(str(path))
    methods = []
    for owner in pe.net.mdtables.TypeDef:
        for entry in owner.MethodList:
            method = entry.row
            if not method.Rva:
                continue
            body = CilMethodBody(CilMethodBodyReaderBytes(pe.get_data(method.Rva)))
            instructions = []
            for instruction in body.instructions:
                record = {
                    "il_offset": hex(instruction.offset - body.header_size),
                    "rva": hex(method.Rva + instruction.offset),
                    "opcode": instruction.opcode.name,
                    "operand": resolve_token(pe, instruction.operand),
                }
                if instruction.opcode.operand_type.name in ("InlineBrTarget", "ShortInlineBrTarget"):
                    record["branch_target_il"] = hex(instruction.operand - body.header_size)
                instructions.append(record)
            methods.append({
                "owner": str(owner.TypeNamespace) + "." + str(owner.TypeName),
                "method": str(method.Name), "metadata_token": hex(0x06000000 | entry.row_index),
                "body_rva": hex(method.Rva), "header_size": body.header_size,
                "instructions": instructions,
            })
    return {"methods": methods}


def native(path, rva, size):
    if not 1 <= size <= 65536:
        raise ValueError("size must be 1..65536")
    pe = pefile.PE(str(path), fast_load=True)
    if pe.FILE_HEADER.Machine != 0x14c:
        raise ValueError("expected x86 PE32")
    raw = pe.get_data(rva, size)
    if len(raw) != size:
        raise ValueError("requested range is not fully file-backed")
    base = pe.OPTIONAL_HEADER.ImageBase
    engine = Cs(CS_ARCH_X86, CS_MODE_32)
    instructions = []
    for ins in engine.disasm(raw, base + rva):
        instructions.append({
            "va": hex(ins.address), "rva": hex(ins.address - base),
            "file_offset": hex(pe.get_offset_from_rva(ins.address - base)),
            "bytes": ins.bytes.hex(), "mnemonic": ins.mnemonic, "operands": ins.op_str,
        })
    return {
        "image_base": hex(base), "requested_rva": hex(rva), "requested_size": size,
        "decoded_size": sum(len(bytes.fromhex(i["bytes"])) for i in instructions),
        "instruction_boundary_verified": False, "instructions": instructions,
    }


def references(path, va):
    pe = pefile.PE(str(path), fast_load=True)
    pattern = struct.pack("<I", va)
    matches = []
    for section in pe.sections:
        if not section.Characteristics & 0x20000000:
            continue
        raw = section.get_data()
        offset = 0
        while True:
            offset = raw.find(pattern, offset)
            if offset < 0:
                break
            rva = section.VirtualAddress + offset
            matches.append({"operand_file_offset": hex(section.PointerToRawData + offset),
                            "operand_rva": hex(rva), "operand_va": hex(pe.OPTIONAL_HEADER.ImageBase + rva)})
            offset += 1
    return {"searched_va": hex(va), "kind": "unverified_absolute_u32_matches_in_executable_sections", "matches": matches}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", type=Path, default=DEFAULT_WORKSPACE)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("launcher")
    disassembly = commands.add_parser("native")
    disassembly.add_argument("--rva", type=lambda s: int(s, 0), required=True)
    disassembly.add_argument("--size", type=lambda s: int(s, 0), required=True)
    refs = commands.add_parser("refs")
    refs.add_argument("--va", type=lambda s: int(s, 0), required=True)
    args = parser.parse_args()
    relative = "CODO_OfflineLauncher.exe" if args.command == "launcher" else "CODOL/codoMP_client_shipRetail.exe"
    path = checked_sample(args.workspace, relative)
    if args.command == "launcher":
        result = launcher(path)
    elif args.command == "native":
        result = native(path, args.rva, args.size)
    else:
        result = references(path, args.va)
    result.update({"schema_version": 1, "sample": relative, "sha256": sha256(path), "execution_performed": False})
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
