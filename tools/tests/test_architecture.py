from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from check_architecture import violations


class ArchitectureTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        for crate in ("server", "protocol"):
            source = self.root / "crates" / crate / "src"
            source.mkdir(parents=True)
            (source / "lib.rs").write_text("")

    def source(self, name, text):
        path = self.root / "crates/server/src" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def test_forbids_direct_grouped_and_relative_dependencies(self):
        for code in (
            "use crate::transport::game;",
            "use crate::{content::maps, transport::{game, lobby}};",
            "use super::super::transport::game as endpoint;",
            "fn run() { super::super::transport::game::run(); }",
        ):
            with self.subTest(code=code):
                self.source("game/example.rs", code)
                self.assertTrue(violations(self.root))

    def test_checks_production_items_after_an_inline_test_module(self):
        self.source("backend/example.rs", """
            #[cfg(test)]
            mod tests { use crate::transport::auth; }
            fn production() { crate::transport::auth::run(); }
        """)
        self.assertEqual(len(violations(self.root)), 1)

    def test_allows_test_only_modules_without_hiding_other_modules(self):
        self.source("backend/example.rs", """
            #[cfg(test)]
            mod integration { use crate::transport::auth; }
            mod production { use crate::contracts::account::AccountId; }
        """)
        self.assertEqual(violations(self.root), [])

    def test_ignores_strings_nested_comments_and_test_markers_inside_strings(self):
        self.source("game/example.rs", '''
            const EXAMPLE: &str = r##"#[cfg(test)] mod tests {
                include_str!("../elsewhere"); #[path = "outside.rs"]
                use crate::transport::auth; }"##;
            /* outer /* inner */ use crate::transport::game; */
            const BRACE: char = '}';
            use crate::content::maps;
        ''')
        self.assertEqual(violations(self.root), [])
        self.source("game/example.rs", '''
            const EXAMPLE: &str = "#[cfg(test)] mod tests {";
            use crate::transport::auth;
        ''')
        self.assertEqual(len(violations(self.root)), 1)

    def test_checks_layout_and_embedded_resource_paths(self):
        self.source("game_new.rs", "")
        self.source("game/example.rs", '''
            #[path = "outside.rs"] mod elsewhere;
            const DATA: &str = include_str!("../../data.json");
        ''')
        self.assertEqual(len(violations(self.root)), 3)

    def test_contracts_cannot_depend_on_business_implementations(self):
        self.source("contracts/example.rs", "use crate::backend::identity;")
        self.assertEqual(len(violations(self.root)), 1)


if __name__ == "__main__":
    unittest.main()
