//! Extract map semantics from the measured 3.3.3.3 mmstart forms.
//! Producer 0x6e6bf0; native formats 0xc7e740/0xc7e668/0xc7e6d0.
//! This parser never executes settings or treats their text as console code.
use anyhow::{Result, bail, ensure};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MapSelection {
    pub map: String,
    pub mode: String,
}

/// The captured training start has no explicit map and returns None. Custom
/// room starts must identify exactly one map and mode. The caller separately
/// binds that choice to a captured map catalog and an authenticated admission.
pub(crate) fn from_mmstart(command: &str) -> Result<Option<MapSelection>> {
    ensure!(
        command.len() <= 4096
            && command.is_ascii()
            && command.bytes().all(|byte| (32..=126).contains(&byte)),
        "invalid mmstart length or control characters"
    );
    let mut statements = command.split(';').map(str::trim).collect::<Vec<_>>();
    // The current native custom format may end with one semicolon. Empty
    // interior statements and repeated trailing separators are rejected.
    if statements.last() == Some(&"") {
        statements.pop();
    }
    ensure!(
        (1..=64).contains(&statements.len()) && statements.iter().all(|s| !s.is_empty()),
        "empty or excessive mmstart statements"
    );
    let tokens = |statement: &str| {
        statement
            .split(' ')
            .filter(|token| !token.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    ensure!(
        tokens(statements[0]) == ["mmstart", "sv_resetServer"],
        "mmstart must begin with the native reset header"
    );

    let training = [
        "mmstart sv_resetServer",
        "2486 1",
        "3265 12",
        "2489 1",
        "playlist 0",
        "start_map_rotate_once 0",
    ];
    if statements.len() == training.len()
        && statements
            .iter()
            .zip(training)
            .all(|(actual, expected)| tokens(actual) == tokens(expected))
    {
        return Ok(None);
    }

    let mut map = None;
    let mut mode = None;
    let mut seen = HashSet::new();
    for statement in statements.into_iter().skip(1) {
        let words = tokens(statement);
        ensure!(
            words.len() == 2,
            "mmstart settings require one paired value"
        );
        let key = words[0].as_str();
        let value = words[1].as_str();
        ensure!(seen.insert(key.to_owned()), "duplicate mmstart setting");
        match key {
            "map" => {
                ensure!(
                    identifier(value) && value.starts_with("mp_") && value.len() > 3,
                    "invalid multiplayer map identifier"
                );
                map = Some(value.to_owned());
            }
            "g_gametype" => {
                ensure!(
                    identifier(value) && value == "war",
                    "unsupported map game mode"
                );
                mode = Some(value.to_owned());
            }
            // These are actual parameters of the two current custom-room
            // formats. Their syntactic validation does not apply the setting.
            "2486" | "3265" => {
                let count = unsigned(value)?;
                ensure!(
                    (1..=32).contains(&count),
                    "mmstart player count outside range"
                );
            }
            "2489" => {
                ensure!(matches!(value, "0" | "1"), "invalid native training flag");
            }
            "2500" => ensure!(value == "0", "unsupported native custom-room flag"),
            "2810" | "g_difficulty" => {
                unsigned(value)?;
            }
            "sv_post_options" | "2654" => {
                // Preserve opaque native/p2p tokens in the admission's original
                // command. A value cannot introduce another setting, quotation,
                // path or script expansion; no opaque token is ever executed.
                ensure!(opaque(value), "invalid opaque native mmstart value");
            }
            // Rotation is a separate native form. Do not let a partly parsed
            // playlist silently fall back to the currently prepared map.
            "playlist" | "start_map_rotate_once" => {
                bail!("unsupported or mixed mmstart rotation request");
            }
            _ => bail!("unsupported mmstart setting"),
        }
    }
    match (map, mode) {
        (Some(map), Some(mode)) => Ok(Some(MapSelection { map, mode })),
        _ => bail!("custom mmstart requires one map and one game mode"),
    }
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() < 32 // matches the native gamestate identifier budget
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn unsigned(value: &str) -> Result<u32> {
    ensure!(
        !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()),
        "native mmstart number must be unsigned decimal"
    );
    value
        .parse()
        .map_err(|_| anyhow::anyhow!("native mmstart number overflow"))
}

fn opaque(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'+' | b'=' | b':')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRAINING: &str =
        "mmstart sv_resetServer; 2486 1; 3265 12; 2489 1; playlist 0; start_map_rotate_once 0";
    const CUSTOM: &str = "mmstart sv_resetServer; 2486 1; 3265 12; 2489 0; 2500 0; g_gametype war; g_difficulty 2; map mp_killhouse_sh;2654 owner_1;";

    #[test]
    fn captured_training_form_has_no_explicit_map() {
        assert_eq!(from_mmstart(TRAINING).unwrap(), None);
        assert_eq!(from_mmstart(&format!("  {TRAINING}; ")).unwrap(), None);
        assert_eq!(from_mmstart(&TRAINING.replace(' ', "  ")).unwrap(), None);
        for changed in [
            TRAINING.replace("playlist 0", "playlist 1"),
            TRAINING.replace("start_map_rotate_once 0", "start_map_rotate_once 1"),
            format!("{TRAINING};map mp_killhouse_sh;g_gametype war"),
            "mmstart sv_resetServer; playlist 0".to_owned(),
        ] {
            assert!(from_mmstart(&changed).is_err());
        }
    }

    #[test]
    fn native_custom_forms_extract_map_and_mode_without_executing_metadata() {
        let expected = Some(MapSelection {
            map: "mp_killhouse_sh".into(),
            mode: "war".into(),
        });
        assert_eq!(from_mmstart(CUSTOM).unwrap(), expected);
        let p2p = "mmstart sv_resetServer;2486 2;3265 12;2489 0;2500 0;g_gametype war;2810 7;sv_post_options AQID==;map mp_killhouse_sh;2654 a-b_0123";
        assert_eq!(from_mmstart(p2p).unwrap(), expected);
        // Selection extraction is independent of the order of custom fields.
        assert_eq!(
            from_mmstart("mmstart sv_resetServer;map mp_killhouse_sh;g_gametype war").unwrap(),
            expected
        );
    }

    #[test]
    fn duplicate_or_incomplete_semantics_never_choose_an_arbitrary_map() {
        for command in [
            "mmstart sv_resetServer; map mp_afghan",
            "mmstart sv_resetServer; g_gametype war",
            "mmstart sv_resetServer; map mp_afghan;map mp_killhouse_sh;g_gametype war",
            "mmstart sv_resetServer; map mp_afghan;map mp_afghan;g_gametype war",
            "mmstart sv_resetServer; map mp_afghan;g_gametype war;g_gametype war",
            "mmstart sv_resetServer; map mp_afghan;g_gametype war;2489 0;2489 1",
        ] {
            assert!(
                from_mmstart(command).is_err(),
                "accepted ambiguous selection"
            );
        }
    }

    #[test]
    fn path_control_separator_and_script_injections_are_rejected() {
        let attacks = [
            "map mp_afghan;quit",
            "map mp_afghan;exec config.cfg",
            "map ../mp_afghan",
            "map mp_afghan/other",
            "map mp_afghan\\other",
            "map \"mp_afghan\"",
            "map mp_afghan other",
            "map mp_afghan\nquit",
            "map mp_afghan\rquit",
            "map mp_afghan\0quit",
            "map mp_afghan\tquit",
            "map mp_afghan$(quit)",
            "map mp_afghan`quit`",
            "map MP_AFGHAN",
            "map mp_",
            "map mp_地图",
            "map mp_afghan;unknown 1",
            "map mp_afghan;2654 ../file",
            "map mp_afghan;sv_post_options /tmp/file",
            "map mp_afghan;sv_post_options AQID== extra",
            "map mp_afghan;;",
        ];
        for attack in attacks {
            let command = format!("mmstart sv_resetServer;g_gametype war;{attack}");
            assert!(
                from_mmstart(&command).is_err(),
                "accepted malformed command"
            );
        }
    }

    #[test]
    fn malformed_native_flags_and_unsupported_modes_fail_before_selection() {
        for extra in [
            "2486 0",
            "3265 33",
            "2489 2",
            "2500 1",
            "2810 -1",
            "g_difficulty 1x",
            "g_difficulty 4294967296",
            "2654",
        ] {
            assert!(
                from_mmstart(&format!(
                    "mmstart sv_resetServer;map mp_afghan;g_gametype war;{extra}"
                ))
                .is_err()
            );
        }
        assert!(from_mmstart("mmstart sv_resetServer;map mp_afghan;g_gametype sd").is_err());
        assert!(from_mmstart("mmstart sv_resetServer;map mp_afghan;g_gametype WAR").is_err());
        assert!(from_mmstart("mmstart quit;map mp_afghan;g_gametype war").is_err());
        assert!(
            from_mmstart(&format!(
                "mmstart sv_resetServer;map mp_{};g_gametype war",
                "a".repeat(29)
            ))
            .is_err()
        );
        assert!(from_mmstart(&" ".repeat(4097)).is_err());
    }
}
