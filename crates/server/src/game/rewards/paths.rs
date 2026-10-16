//! Target-map authored routes. Missing speeds remain absent; graph topology
//! does not establish movement parameters, collision clearance or gameplay.
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FlightNode {
    pub name: String,
    pub origin: [f32; 3],
    #[serde(default)]
    pub angles: Option<[f32; 3]>,
    pub target: Option<usize>,
    pub script_airspeed: Option<f32>,
    pub script_accel: Option<f32>,
    pub script_delay: Option<f32>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FlightNodes {
    pub sources_sha256: [String; 1],
    pub nodes: Vec<FlightNode>,
    pub entries: BTreeMap<String, Vec<usize>>,
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Route {
    pub nodes: Vec<usize>,
    /// A loop can begin after its initial entry pointer, as in mp_afghan.
    pub repeat_from: Option<usize>,
}
impl FlightNodes {
    pub fn valid(&self) -> bool {
        let n = self.nodes.len();
        (1..=256).contains(&n)
            && self
                .sources_sha256
                .iter()
                .all(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            && !self.entries.is_empty()
            && self.nodes.iter().all(|node| {
                !node.name.is_empty()
                    && node.name.len() <= 64
                    && node.name.is_ascii()
                    && node
                        .origin
                        .iter()
                        .all(|v| v.is_finite() && v.abs() <= 30000.0)
                    && node.target.is_none_or(|i| i < n)
                    && node
                        .angles
                        .is_none_or(|a| a.iter().all(|v| v.is_finite() && v.abs() <= 3600.0))
                    && [node.script_airspeed, node.script_accel]
                        .into_iter()
                        .flatten()
                        .all(|v| v.is_finite() && (0.0..=50000.0).contains(&v))
                    && node
                        .script_delay
                        .is_none_or(|v| v.is_finite() && (0.0..=600.0).contains(&v))
            })
            && self.entries.iter().all(|(name, starts)| {
                matches!(
                    name.as_str(),
                    "heli_start"
                        | "heli_loop_start"
                        | "heli_leave"
                        | "heli_crash_start"
                        | "heli_attack_area"
                        | "heli_dest"
                ) && !starts.is_empty()
                    && starts.len() <= n
                    && starts.iter().copied().collect::<BTreeSet<_>>().len() == starts.len()
                    && starts.iter().all(|&i| i < n && self.nodes[i].name == *name)
                    && starts.iter().all(|&i| {
                        self.route(i).is_some_and(|r| {
                            if name == "heli_loop_start" {
                                r.repeat_from.is_some()
                            } else {
                                r.repeat_from.is_none()
                            }
                        })
                    })
            })
    }
    pub fn route(&self, start: usize) -> Option<Route> {
        let mut nodes = Vec::new();
        let mut next = Some(start);
        while let Some(index) = next {
            let node = self.nodes.get(index)?;
            if let Some(repeat_from) = nodes.iter().position(|&i| i == index) {
                return Some(Route {
                    nodes,
                    repeat_from: Some(repeat_from),
                });
            }
            nodes.push(index);
            next = node.target;
        }
        Some(Route {
            nodes,
            repeat_from: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authored_loop_keeps_entry_prefix_and_missing_speeds_without_unrolling() {
        let mut paths: FlightNodes = serde_json::from_value(serde_json::json!({
            "sources_sha256": ["0".repeat(64)],
            "entries": {"heli_loop_start": [0]},
            "nodes": [
                {"name": "heli_loop_start", "origin": [0,0,500], "target": 1},
                {"name": "auto1", "origin": [100,0,500], "target": 2, "script_airspeed": 40, "script_accel": 15},
                {"name": "auto2", "origin": [100,100,500], "target": 1, "script_delay": 2}
            ]
        })).unwrap();
        assert!(paths.valid());
        assert!(paths.nodes[0].script_airspeed.is_none());
        assert_eq!(
            paths.route(0),
            Some(Route {
                nodes: vec![0, 1, 2],
                repeat_from: Some(1)
            })
        );
        paths.nodes[2].target = Some(3);
        assert!(!paths.valid());
        assert!(paths.route(0).is_none());
        paths.nodes[2].target = None;
        assert!(!paths.valid());
    }
    #[test]
    fn heading_is_optional_but_nonfinite_or_unbounded_values_are_rejected() {
        let mut paths: FlightNodes = serde_json::from_value(serde_json::json!({
            "sources_sha256":["0".repeat(64)],"entries":{"heli_start":[0]},
            "nodes":[{"name":"heli_start","origin":[0,0,500],"target":null,"angles":[0,135,0]}]
        }))
        .unwrap();
        assert!(paths.valid());
        assert_eq!(paths.nodes[0].angles, Some([0.0, 135.0, 0.0]));
        for invalid in [f32::NAN, f32::INFINITY, 3601.0] {
            paths.nodes[0].angles = Some([0.0, invalid, 0.0]);
            assert!(!paths.valid());
        }
        paths.nodes[0].angles = None;
        assert!(paths.valid());
    }
}
