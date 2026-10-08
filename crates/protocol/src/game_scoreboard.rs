//! Current 3.3.3.3 scoreboard updates through binary reliable command J.
//! Opcode 2/reader 0x46e470 stores the binary payload; 0x57e330 routes J to
//! 0x57bf70, then 0x57b760 rebuilds the cached rows read by the HUD and Tab.
//! Offsets below belong to captured mp/playerdata.def version 233 and exclude
//! the persisted file's four-byte CRC. They never select an old-build layout.
use crate::lobby::LobbyError;
use std::collections::{BTreeMap, BTreeSet};

pub const BINARY_PAYLOAD_LIMIT: usize = 0x3fc;
pub const PLAYERDATA_VERSION: u32 = 233;
pub const PLAYERDATA_RECORD_BYTES: usize = 14426;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScoreboardRow {
    pub client_number: u8,
    pub score: i32,
    /// Native connecting clients use -1; Bot ping can be 0.
    pub ping: i32,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub headshots: u32,
    pub objectives: u32,
    pub downs: u32,
    pub revives: u32,
    pub skill: i32,
    /// 0 means no icon; 1..=8 resolve the current status-icon configstrings.
    pub status_icon: u8,
    pub killstreak: u32,
    pub xp: u32,
    pub gp: u32,
    pub qq_membership: u32,
    pub netcafe_privileges: u32,
    pub user_id: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScoreboardSnapshot {
    /// Use actual connected client numbers. Native rebuilding requires their
    /// clientInfo records to be valid; it obtains names, teams and rank there.
    pub rows: Vec<ScoreboardRow>,
    /// The captured aarTeamScores array has four entries. Native rebuilding
    /// reads indices 1 and 2 for the two teams.
    pub team_scores: [i32; 4],
    pub score_limit: u32,
}

// Order: assists, deaths, downs, gp, headshots, kills, killstreak,
// netcafe_privileges, objectives, ping, qq_membership, revives, score,
// skill, status_icon, xp, dw_userID_lo, dw_userID_hi.
const ROW_ZERO_OFFSETS: [u16; 18] = [
    67, 137, 141, 145, 149, 158, 162, 9426, 231, 235, 9430, 240, 244, 252, 256, 261, 9630, 9626,
];

fn row_offset(client: u8, field: usize) -> u16 {
    // The core fields and later profile extensions occupy separate spans.
    // Every one of the 19 slots is verified against the captured definition.
    let stride = if matches!(field, 7 | 10 | 16 | 17) {
        8
    } else {
        198
    };
    ROW_ZERO_OFFSETS[field] + u16::from(client) * stride
}

fn row_values(row: ScoreboardRow) -> [u32; 18] {
    [
        row.assists,
        row.deaths,
        row.downs,
        row.gp,
        row.headshots,
        row.kills,
        row.killstreak,
        row.netcafe_privileges,
        row.objectives,
        row.ping as u32,
        row.qq_membership,
        row.revives,
        row.score as u32,
        row.skill as u32,
        u32::from(row.status_icon),
        row.xp,
        row.user_id as u32,
        (row.user_id >> 32) as u32,
    ]
}

/// Return binary payloads beginning with J, ready for server-message opcode 2.
/// Each payload is J followed by complete [offset LE32, byte count u8, bytes]
/// records. The transport owns their shared reliable sequences and ACKs.
///
/// These payloads may contain NUL and must not pass through a text-command
/// writer. The native reliable ring stores [!, NUL, length LE16, payload], so
/// its C-string XOR key is just !. The wire contains the length and payload,
/// without that ring header. Resending the same sequence is idempotent.
///
/// Only measured scoreboard numeric fields are written. Backpack, clothing,
/// names and other profile bytes retain their existing values. Send rows at
/// initial world readiness and whenever authoritative match statistics change.
pub fn binary_commands(board: &ScoreboardSnapshot) -> Result<Vec<Vec<u8>>, LobbyError> {
    let mut seen = BTreeSet::new();
    if board.rows.len() > 19 || board.score_limit > i32::MAX as u32 {
        return Err(LobbyError("invalid current scoreboard size or limit"));
    }
    for row in &board.rows {
        if row.client_number >= 19
            || !seen.insert(row.client_number)
            || row.ping < -1
            || row.status_icon > 8
            || [
                row.kills,
                row.deaths,
                row.assists,
                row.headshots,
                row.objectives,
                row.downs,
                row.revives,
                row.killstreak,
                row.xp,
                row.gp,
                row.qq_membership,
                row.netcafe_privileges,
            ]
            .iter()
            .any(|n| *n > i32::MAX as u32)
        {
            return Err(LobbyError("invalid current scoreboard row"));
        }
    }

    let mut fields = BTreeMap::new();
    fields.insert(63, board.score_limit.to_le_bytes());
    for (team, score) in board.team_scores.iter().enumerate() {
        fields.insert(9778 + team as u16 * 4, score.to_le_bytes());
    }
    for &row in &board.rows {
        for (field, value) in row_values(row).into_iter().enumerate() {
            fields.insert(row_offset(row.client_number, field), value.to_le_bytes());
        }
    }

    // Merge only adjacent bytes of explicitly owned fields. In particular,
    // never bridge the profile's packed bool, strings or unselected counters.
    let mut ranges: Vec<(u16, Vec<u8>)> = Vec::new();
    for (offset, bytes) in fields {
        if let Some((start, run)) = ranges.last_mut()
            && usize::from(*start) + run.len() == usize::from(offset)
            && run.len() + bytes.len() <= u8::MAX as usize
        {
            run.extend_from_slice(&bytes);
        } else {
            ranges.push((offset, bytes.to_vec()));
        }
    }
    let mut commands = vec![vec![b'J']];
    for (offset, bytes) in ranges {
        if usize::from(offset) + bytes.len() > PLAYERDATA_RECORD_BYTES {
            return Err(LobbyError("scoreboard field outside captured playerdata"));
        }
        if commands.last().expect("one payload").len() + 5 + bytes.len() > BINARY_PAYLOAD_LIMIT {
            commands.push(vec![b'J']);
        }
        let command = commands.last_mut().expect("one payload");
        command.extend_from_slice(&u32::from(offset).to_le_bytes());
        command.push(bytes.len() as u8);
        command.extend_from_slice(&bytes);
    }
    Ok(commands)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYS: [&str; 18] = [
        "assists",
        "deaths",
        "downs",
        "gp",
        "headshots",
        "kills",
        "killstreak",
        "netcafe_privileges",
        "objectives",
        "ping",
        "qq_membership",
        "revives",
        "score",
        "skill",
        "status_icon",
        "xp",
        "dw_userID_lo",
        "dw_userID_hi",
    ];

    fn fixture() -> serde_json::Value {
        let f: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/protocol/scoreboard-playerdata-3.3.3.3.json"
        ))
        .unwrap();
        assert_eq!(f["client_sha256"], crate::CLIENT_SHA256);
        assert_eq!(f["definition_version"], PLAYERDATA_VERSION);
        assert_eq!(f["definition_size"], PLAYERDATA_RECORD_BYTES);
        assert_eq!(f["binary_server_opcode"], 2);
        assert_eq!(f["payload_opcode"], b'J');
        assert_eq!(f["payload_maximum"], BINARY_PAYLOAD_LIMIT);
        assert_eq!(f["cache"]["base_cg_offset"], 0xbc8c0);
        assert_eq!(f["cache"]["row_stride"], 0x5c);
        f
    }

    // Match 0x57bf70's byte records independently of encoder helpers. The
    // native receiver enforces a 0x7ffc record capacity, not a text terminator.
    fn apply(payload: &[u8], profile: &mut [u8]) {
        assert_eq!(payload[0], b'J');
        assert!((2..=1020).contains(&payload.len()));
        let mut cursor = 1;
        while cursor != payload.len() {
            let offset =
                u32::from_le_bytes(payload[cursor..cursor + 4].try_into().unwrap()) as usize;
            let count = usize::from(payload[cursor + 4]);
            cursor += 5;
            assert!(offset < 0x7ffc && count > 0 && offset + count <= 0x7ffc);
            profile[offset..offset + count].copy_from_slice(&payload[cursor..cursor + count]);
            cursor += count;
        }
    }

    fn field(f: &serde_json::Value, name: &str) -> usize {
        assert_eq!(f["fields"][name]["type"], 0);
        assert_eq!(f["fields"][name]["width"], 4);
        f["fields"][name]["record_offset"].as_u64().unwrap() as usize
    }

    #[test]
    fn every_current_client_offset_matches_the_captured_definition() {
        let f = fixture();
        for client in 0..19 {
            for (column, key) in KEYS.iter().enumerate() {
                assert_eq!(
                    usize::from(row_offset(client, column)),
                    field(&f, &format!("aarScoreboard[{client}].{key}"))
                );
            }
        }
        assert_eq!(field(&f, "aarScoreLimit"), 63);
        for team in 0..4 {
            assert_eq!(
                field(&f, &format!("aarTeamScores[{team}]")),
                9778 + team * 4
            );
        }
    }

    #[test]
    fn local_and_bot_rows_decode_kda_scores_uid_and_preserve_other_profile_fields() {
        let board = ScoreboardSnapshot {
            rows: vec![
                ScoreboardRow {
                    client_number: 0,
                    score: 500,
                    kills: 5,
                    deaths: 2,
                    assists: 3,
                    headshots: 1,
                    user_id: 0xfffe_fdfc_fbf9_f8f7,
                    ..Default::default()
                },
                ScoreboardRow {
                    client_number: 1,
                    score: -20,
                    ping: -1,
                    kills: 2,
                    deaths: 5,
                    status_icon: 1,
                    ..Default::default()
                },
            ],
            team_scores: [0, 2, 5, 0],
            score_limit: 100,
        };
        let commands = binary_commands(&board).unwrap();
        assert_eq!(commands.len(), 1);
        assert!(commands[0].contains(&0));
        let f = fixture();
        let mut profile = vec![0xa6; 0x7ffc];
        let before = profile.clone();
        apply(&commands[0], &mut profile);
        let read = |name: &str| {
            let at = field(&f, name);
            i32::from_le_bytes(profile[at..at + 4].try_into().unwrap())
        };
        for (name, value) in [
            ("aarScoreboard[0].score", 500),
            ("aarScoreboard[0].kills", 5),
            ("aarScoreboard[0].deaths", 2),
            ("aarScoreboard[0].assists", 3),
            ("aarScoreboard[0].headshots", 1),
            ("aarScoreboard[1].score", -20),
            ("aarScoreboard[1].ping", -1),
            ("aarScoreboard[1].kills", 2),
            ("aarScoreboard[1].deaths", 5),
            ("aarScoreboard[1].status_icon", 1),
            ("aarTeamScores[1]", 2),
            ("aarTeamScores[2]", 5),
            ("aarScoreLimit", 100),
        ] {
            assert_eq!(read(name), value);
        }
        assert_eq!(read("aarScoreboard[0].dw_userID_lo") as u32, 0xfbf9_f8f7);
        assert_eq!(read("aarScoreboard[0].dw_userID_hi") as u32, 0xfffe_fdfc);
        let mut owned = BTreeSet::new();
        for row in &board.rows {
            for key in KEYS {
                let offset = field(&f, &format!("aarScoreboard[{}].{key}", row.client_number));
                owned.extend(offset..offset + 4);
            }
        }
        for name in [
            "aarScoreLimit",
            "aarTeamScores[0]",
            "aarTeamScores[1]",
            "aarTeamScores[2]",
            "aarTeamScores[3]",
        ] {
            let offset = field(&f, name);
            owned.extend(offset..offset + 4);
        }
        for at in 0..profile.len() {
            if !owned.contains(&at) {
                assert_eq!(profile[at], before[at], "unowned profile byte {at}");
            }
        }
        let applied = profile.clone();
        apply(&commands[0], &mut profile);
        assert_eq!(profile, applied);
    }

    #[test]
    fn nineteen_rows_split_into_complete_native_records_and_reconstruct_every_row() {
        let board = ScoreboardSnapshot {
            rows: (0..19)
                .rev()
                .map(|client| ScoreboardRow {
                    client_number: client,
                    score: i32::from(client) * 100,
                    kills: u32::from(client) + 1,
                    deaths: u32::from(client) + 2,
                    assists: u32::from(client) + 3,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let commands = binary_commands(&board).unwrap();
        assert!(commands.len() > 1);
        let mut profile = vec![0xa6; 0x7ffc];
        for command in &commands {
            apply(command, &mut profile);
        }
        let f = fixture();
        for client in 0..19 {
            for (key, expected) in [
                ("score", client * 100),
                ("kills", client + 1),
                ("deaths", client + 2),
                ("assists", client + 3),
            ] {
                let at = field(&f, &format!("aarScoreboard[{client}].{key}"));
                assert_eq!(
                    i32::from_le_bytes(profile[at..at + 4].try_into().unwrap()),
                    expected
                );
            }
        }
        let mut reversed = board;
        reversed.rows.reverse();
        assert_eq!(binary_commands(&reversed).unwrap(), commands);
    }

    #[test]
    fn malformed_rows_are_rejected_before_any_command_is_returned() {
        for row in [
            ScoreboardRow {
                client_number: 19,
                ..Default::default()
            },
            ScoreboardRow {
                ping: -2,
                ..Default::default()
            },
            ScoreboardRow {
                status_icon: 9,
                ..Default::default()
            },
            ScoreboardRow {
                kills: i32::MAX as u32 + 1,
                ..Default::default()
            },
        ] {
            assert!(
                binary_commands(&ScoreboardSnapshot {
                    rows: vec![row],
                    ..Default::default()
                })
                .is_err()
            );
        }
        assert!(
            binary_commands(&ScoreboardSnapshot {
                rows: vec![Default::default(); 2],
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            binary_commands(&ScoreboardSnapshot {
                score_limit: u32::MAX,
                ..Default::default()
            })
            .is_err()
        );
    }
}
