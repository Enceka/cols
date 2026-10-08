//! Parse the captured 3.3.3.3 in-game backpack request without executing text.
//! The live request was `mr 1 1 custom1`. Native menu emitters bind argument
//! one to configstring 3 and resolve argument two through the 4291/64 bank.
use anyhow::{Result, ensure};

pub(crate) const BACKPACK_MENU_SLOT: u8 = 1;
pub(crate) const BACKPACKS_PER_CHARACTER: u8 = 5;
pub(crate) const CHARACTER_COUNT: u8 = 12;

/// Return a zero-based backpack slot for this captured menu, or None for a
/// different client-command verb. A malformed/stale `mr` is rejected; callers
/// may ACK receipt while declining the selection. No command is executed and
/// no profile, equipment, pending selection, or reliable sequence is changed.
pub(crate) fn from_client_command(command: &[u8], current_server_id: u32) -> Result<Option<u8>> {
    let words = command
        .split(u8::is_ascii_whitespace)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    if words.first().copied() != Some(b"mr") {
        return Ok(None);
    }
    ensure!(
        command.len() < 1024
            && command
                .iter()
                .all(|byte| byte.is_ascii_graphic() || matches!(byte, b' ' | b'\t')),
        "invalid backpack menu command length or characters"
    );
    ensure!(
        words.len() == 4,
        "backpack menu request requires four tokens"
    );
    ensure!(
        !words[1].is_empty() && words[1].iter().all(u8::is_ascii_digit),
        "backpack menu server ID must be unsigned decimal"
    );
    let server_id = std::str::from_utf8(words[1])?.parse::<u32>()?;
    ensure!(
        server_id == current_server_id,
        "backpack menu request belongs to a different server state"
    );
    ensure!(
        words[2] == BACKPACK_MENU_SLOT.to_string().as_bytes(),
        "backpack menu request has the wrong menu slot"
    );
    let slot = match words[3] {
        b"custom1" => 0,
        b"custom2" => 1,
        b"custom3" => 2,
        b"custom4" => 3,
        b"custom5" => 4,
        _ => anyhow::bail!("unsupported in-game backpack response"),
    };
    Ok(Some(slot))
}

/// Native 0x6bf930 adds the selected slot to currentCharacterIndex * 5 before
/// reading weaponClasses. Use the character from the freshly validated public
/// profile, not a UI-cache guess. All offsets for the resulting class must be
/// selected from the captured schema; the 60 records lack one uniform stride.
pub(crate) fn global_class_index(character_index: u8, backpack_slot: u8) -> Result<u8> {
    ensure!(
        character_index < CHARACTER_COUNT && backpack_slot < BACKPACKS_PER_CHARACTER,
        "character or backpack slot outside captured class array"
    );
    Ok(character_index * BACKPACKS_PER_CHARACTER + backpack_slot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_custom_menu_selects_the_five_zero_based_backpacks() {
        assert_eq!(from_client_command(b"mr 1 1 custom1", 1).unwrap(), Some(0));
        for displayed in 1..=5 {
            let command = format!("mr 17 1 custom{displayed}");
            assert_eq!(
                from_client_command(command.as_bytes(), 17).unwrap(),
                Some(displayed - 1)
            );
        }
        assert_eq!(
            from_client_command(b" \tmr   17  1 custom3\t ", 17).unwrap(),
            Some(2)
        );
        assert_eq!(
            from_client_command(b"mr 65537 1 custom5", 65537).unwrap(),
            Some(4)
        );
    }

    #[test]
    fn stale_wrong_menu_and_noncanonical_responses_cannot_select_a_backpack() {
        for command in [
            "mr 2 1 custom1",
            "mr 257 1 custom1", // Full server ID must match, not only its low byte.
            "mr 1 0 custom1",
            "mr 1 2 custom1",
            "mr 1 64 custom1",
            "mr 1 -1 custom1",
            "mr 1 01 custom1",
            "mr 1 1 custom0",
            "mr 1 1 custom6",
            "mr 1 1 custom60",
            "mr 1 1 custom01",
            "mr 1 1 class1",
            "mr 1 1 CUSTOM1",
            "mr 1 1 custom1 extra",
            "mr 1 1",
            "mr 1",
            "mr",
        ] {
            assert!(
                from_client_command(command.as_bytes(), 1).is_err(),
                "{command}"
            );
        }
    }

    #[test]
    fn control_injection_overflow_and_binary_input_are_rejected_as_menu_requests() {
        for command in [
            b"mr 1 1 custom1\0".as_slice(),
            b"mr 1 1 custom1\n",
            b"mr\n1 1 custom1",
            b"mr 1 1 custom1;quit",
            b"mr 1 1 \"custom1\"",
            b"mr 1 1 custom1\x7f",
            b"mr 1 1 custom\xff",
            b"mr -1 1 custom1",
            b"mr +1 1 custom1",
            b"mr 0x1 1 custom1",
            b"mr 4294967296 1 custom1",
            b"mr 999999999999999999999999999999999 1 custom1",
        ] {
            assert!(from_client_command(command, 1).is_err(), "{command:?}");
        }
        let excessive = format!("mr {} 1 custom1", "0".repeat(1024));
        assert!(from_client_command(excessive.as_bytes(), 0).is_err());
    }

    #[test]
    fn unrelated_commands_have_no_class_selection_semantics() {
        for command in [
            b"".as_slice(),
            b"s",
            b"score",
            b"nextdl 1",
            b"mr_old 1 1 custom1",
            b"say custom1",
            b"\xff\0",
        ] {
            assert_eq!(from_client_command(command, 1).unwrap(), None);
        }
    }

    #[test]
    fn character_local_slot_maps_to_each_of_the_sixty_native_classes() {
        let mut all = std::collections::BTreeSet::new();
        for character in 0..12 {
            for slot in 0..5 {
                let index = global_class_index(character, slot).unwrap();
                assert!(all.insert(index));
            }
        }
        assert_eq!(all, (0..60).collect());
        assert_eq!(global_class_index(0, 0).unwrap(), 0);
        assert_eq!(global_class_index(1, 0).unwrap(), 5);
        assert_eq!(global_class_index(11, 4).unwrap(), 59);
        assert!(global_class_index(12, 0).is_err());
        assert!(global_class_index(0, 5).is_err());
        assert!(global_class_index(255, 255).is_err());
    }
}
