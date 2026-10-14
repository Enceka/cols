//! A single native story start, without a gameplay world or TDM fallback.
use anyhow::{Result, bail, ensure};
use std::collections::HashSet;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Selection {
    pub map: String,
    pub difficulty: Option<u8>,
}

pub(super) fn selection(command: &str, expected_map: &str) -> Result<Selection> {
    ensure!(
        expected_map == codol_protocol::story::FIRST_MAP,
        "unsupported story capture map"
    );
    ensure!(
        command.len() <= 4096
            && command.is_ascii()
            && command.bytes().all(|b| (32..=126).contains(&b)),
        "invalid story start text"
    );
    let mut statements = command.split(';').map(str::trim).filter(|s| !s.is_empty());
    ensure!(
        statements.next() == Some("mmstart sv_resetServer"),
        "invalid story reset header"
    );
    let (mut map, mut mode, mut difficulty) = (None, None, None);
    let mut seen = HashSet::new();
    for statement in statements {
        let words: Vec<_> = statement.split_whitespace().collect();
        // Native cargo capture emits an empty trailing2654 option. Preserve
        // exactly that form; every other setting still needs one value.
        if words == ["2654"] {
            ensure!(seen.insert("2654"), "duplicate story option");
            continue;
        }
        ensure!(
            words.len() == 2 && seen.insert(words[0]),
            "invalid or duplicate story setting"
        );
        let value = words[1];
        match words[0] {
            "map" => {
                ensure!(value == expected_map, "story capture map differs");
                map = Some(value);
            }
            "g_gametype" => {
                ensure!(value == "coop", "story capture requires coop");
                mode = Some(value);
            }
            "2486" | "3265" => ensure!(
                value == "1",
                "story capture requires native single-player limits"
            ),
            "2489" => ensure!(matches!(value, "0" | "1"), "invalid native start flag"),
            "2500" => ensure!(value == "0", "unsupported custom start flag"),
            "g_difficulty" | "2810" => {
                ensure!(
                    difficulty.is_none() && value.bytes().all(|b| b.is_ascii_digit()),
                    "invalid duplicate difficulty"
                );
                difficulty = Some(value.parse::<u8>()?);
            }
            "sv_post_options" | "2654" => ensure!(
                value.len() <= 256
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-.:,+/=".contains(&b)),
                "invalid opaque story option"
            ),
            _ => bail!("unsupported story start setting"),
        }
    }
    ensure!(
        map.is_some() && mode.is_some(),
        "story start must identify map and coop mode"
    );
    Ok(Selection {
        map: map.unwrap().to_owned(),
        difficulty,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const START: &str = "mmstart sv_resetServer; 2486 1; 3265 1; 2489 0; 2500 0; g_gametype coop; g_difficulty 1; map coop_training_mp_cargoship;2654 A_-==;";

    #[test]
    fn explicit_story_start_preserves_selection_without_executing_native_options() {
        assert_eq!(
            selection(START, codol_protocol::story::FIRST_MAP).unwrap(),
            Selection {
                map: codol_protocol::story::FIRST_MAP.into(),
                difficulty: Some(1),
            }
        );
        assert!(
            selection(
                &START.replace("g_difficulty", "2810"),
                codol_protocol::story::FIRST_MAP
            )
            .is_ok()
        );
    }

    #[test]
    fn native_cargo_start_accepts_only_the_captured_empty_trailing_option() {
        let start = "mmstart sv_resetServer; 2486 1; 3265 1; 2489 0; 2500 0; g_gametype coop;  g_difficulty 0; map coop_training_mp_cargoship;2654 ;";
        let selected = selection(start, codol_protocol::story::FIRST_MAP).unwrap();
        assert_eq!(selected.difficulty, Some(0));
        for invalid in [
            start.replace("2654 ;", "2654;2654;"),
            start.replace("2654 ;", "g_difficulty;"),
            start.replace("2654 ;", "quit;"),
        ] {
            assert!(selection(&invalid, codol_protocol::story::FIRST_MAP).is_err());
        }
    }

    #[test]
    fn story_capture_rejects_other_maps_modes_capacity_and_injected_commands() {
        for text in [
            START.replace("coop_training_mp_cargoship", "mp_afghan"),
            START.replace("g_gametype coop", "g_gametype war"),
            START.replace("3265 1", "3265 12"),
            START.replace("g_gametype coop;", ""),
            START.replace("g_difficulty 1", "g_difficulty 256"),
            format!("{START} quit"),
            format!("{START} map coop_training_mp_cargoship"),
            format!("{START} 2810 1"),
            "mmstart sv_resetServer; playlist 0; start_map_rotate_once 0".into(),
            START.replace("A_-==", "$(id)"),
        ] {
            assert!(
                selection(&text, codol_protocol::story::FIRST_MAP).is_err(),
                "{text}"
            );
        }
    }
}
