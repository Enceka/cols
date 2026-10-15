//! 3.3.3.3 coGameTypesTable tokens; native aliases are not wire gametypes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TeamMode {
    #[default]
    Tactical,
    Assault,
}
impl TeamMode {
    pub(crate) fn from_native_token(token: &str) -> Option<Self> {
        match token {
            "war" => Some(Self::Tactical),
            "cha" => Some(Self::Assault),
            _ => None,
        }
    }
    pub(crate) fn native_number(self) -> u32 {
        match self {
            Self::Tactical => 1,
            Self::Assault => 15,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_war_alias_is_tdm_while_charge_has_its_own_token_and_chinese_label() {
        let capture: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/team-modes-3.3.3.3.json"
        )))
        .unwrap();
        assert_eq!(capture["client_sha256"], codol_protocol::CLIENT_SHA256);
        for (token, mode, label) in [
            ("war", TeamMode::Tactical, "MPUI_TEAM_DEATHMATCH"),
            ("cha", TeamMode::Assault, "MPUI_CHA"),
        ] {
            let row = capture["mode_rows"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r[4] == token)
                .unwrap();
            assert_eq!(
                row[3].as_str().unwrap().parse::<u32>().unwrap(),
                mode.native_number()
            );
            assert_eq!(row[6], label);
            assert_eq!(TeamMode::from_native_token(token), Some(mode));
            assert_eq!(row[5], if token == "war" { "tdm" } else { "" });
        }
        assert_eq!(
            capture["localizations"]["MPUI_TEAM_DEATHMATCH"]["value_gb18030"],
            "战术团队竞技模式"
        );
        assert_eq!(
            capture["localizations"]["MPUI_CHA"]["value_gb18030"],
            "冲锋团队竞技"
        );
        assert_eq!(TeamMode::from_native_token("tdm"), None);
        assert_eq!(TeamMode::from_native_token("clash"), None);
    }
}
