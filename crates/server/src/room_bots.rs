//! Lobby participation for the eleven players simulated by TdmMatch.
use std::collections::BTreeSet;

#[derive(Clone)]
pub(crate) struct Policy {
    pub version: u32,
    pub maps: BTreeSet<String>,
}
impl Policy {
    pub fn accepts(&self, info: &codol_protocol::room_messages::RoomInfo) -> bool {
        info.playlist == 255
            && info.mode == "war"
            && info.maximum == 12
            && info.minimum <= 12
            && self.maps.contains(&info.map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bots_only_join_supported_custom_tdm_with_the_real_roster_capacity() {
        let policy = Policy {
            version: 7,
            maps: BTreeSet::from(["mp_afghan".into(), "mp_killhouse_sh".into()]),
        };
        let mut info = codol_protocol::room_messages::RoomInfo {
            mode: "war".into(),
            map: "mp_killhouse_sh".into(),
            playlist: 255,
            minimum: 4,
            maximum: 12,
        };
        assert!(policy.accepts(&info));
        info.playlist = 0;
        assert!(!policy.accepts(&info));
        info.playlist = 255;
        info.mode = "sd".into();
        assert!(!policy.accepts(&info));
        info.mode = "war".into();
        info.map = "mp_killhouse".into();
        assert!(!policy.accepts(&info));
        info.map = "mp_afghan".into();
        info.maximum = 18;
        assert!(!policy.accepts(&info));
        info.maximum = 12;
        info.minimum = 13;
        assert!(!policy.accepts(&info));
    }
}
