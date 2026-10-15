//! Native 3.3.3.3 room browsing and channel admission wire formats.
//! Query 4: 0x702520; paged builder: 0xa570d0; join builder: 0xa4ca40.
//! These codecs do not enable browsing or cross-client membership by themselves.
use crate::lobby::channel::LobbyError;
use crate::lobby::task::Reader;
use crate::lobby::task::Request;

/// Local result budget, shared by ordinary and paged browser requests.
pub const MAX_BROWSER_RESULTS: u32 = 50;

#[derive(Debug, PartialEq, Eq)]
pub enum Position {
    Offset(u32),
    /// 0xa558b0 copies the last result's session ID into the next request.
    /// The initial page also carries eight bytes; preserve them as opaque data.
    Page {
        initial: bool,
        cursor: [u8; 8],
    },
}

#[derive(Debug, PartialEq, Eq)]
pub struct Search {
    pub position: Position,
    pub maximum: u32,
    /// Exactly four Int32 values in this build (0x702520), not the six used
    /// by the older AlphaBuild. Only the first is named here: the build value.
    pub criteria: [i32; 4],
}
impl Search {
    pub fn parse(request: &Request<'_>) -> Result<Self, LobbyError> {
        if request.service != 21 || !matches!(request.task, 5 | 13) {
            return Err(LobbyError("expected room browser search task"));
        }
        let mut r = Reader::new(request.arguments);
        if r.u32()? != 4 {
            return Err(LobbyError("expected room browser query 4"));
        }
        let position = if request.task == 5 {
            Position::Offset(r.u32()?)
        } else {
            let initial = r.boolean()?;
            let cursor = r
                .blob()?
                .try_into()
                .map_err(|_| LobbyError("unexpected browser cursor size"))?;
            Position::Page { initial, cursor }
        };
        let maximum = r.u32()?;
        if maximum == 0 || maximum > MAX_BROWSER_RESULTS {
            return Err(LobbyError("browser page exceeds local result budget"));
        }
        let mut criteria = [0; 4];
        for criterion in &mut criteria {
            *criterion = r.i32()?;
        }
        r.finish()?;
        Ok(Self {
            position,
            maximum,
            criteria,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ChannelJoin {
    pub channel_id: u64,
    /// The normal JoinRoom path (0xa4db00) supplies zero for this second UInt64.
    /// Other credential semantics require separate evidence and admission policy.
    pub credential: u64,
}
impl ChannelJoin {
    pub fn parse(arguments: &[u8]) -> Result<Self, LobbyError> {
        let mut r = Reader::new(arguments);
        let channel_id = r.u64()?;
        let credential = r.u64()?;
        r.finish()?;
        Ok(Self {
            channel_id,
            credential,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::task::Writer;
    use crate::room::description::Description;

    fn fixture() -> serde_json::Value {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/room-browser-synthetic.json"
        )))
        .unwrap()
    }
    fn bytes(value: &serde_json::Value) -> Vec<u8> {
        let hex = value.as_str().unwrap();
        assert!(hex.len().is_multiple_of(2));
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn browser_requests_match_independent_native_layout_vectors() {
        let f = fixture();
        for (name, position) in [
            ("ordinary", Position::Offset(0)),
            (
                "first_page",
                Position::Page {
                    initial: true,
                    cursor: [0; 8],
                },
            ),
            (
                "next_page",
                Position::Page {
                    initial: false,
                    cursor: [1, 2, 3, 4, 5, 6, 7, 8],
                },
            ),
        ] {
            let wire = bytes(&f["requests"][name]);
            let request = Request::parse(&wire).unwrap();
            let search = Search::parse(&request).unwrap();
            assert_eq!(search.position, position);
            assert_eq!(search.maximum, 20);
            assert_eq!(search.criteria, [-688906341, 0, 16, 0]);
            for end in 3..wire.len() {
                let truncated = Request::parse(&wire[..end]).unwrap();
                assert!(Search::parse(&truncated).is_err(), "{name} at {end}");
            }
        }
    }

    #[test]
    fn browser_rejects_other_tasks_queries_criteria_and_invalid_page_fields() {
        for (service, task, query, initial, cursor_size, maximum, count) in [
            (11, 13, 4, 1, 8, 20, 4),
            (21, 12, 4, 1, 8, 20, 4),
            (21, 13, 3, 1, 8, 20, 4),
            (21, 13, 4, 2, 8, 20, 4),
            (21, 13, 4, 1, 7, 20, 4),
            (21, 13, 4, 1, 9, 20, 4),
            (21, 13, 4, 1, 8, 0, 4),
            (21, 13, 4, 1, 8, 51, 4),
            (21, 13, 4, 1, 8, 20, 3),
            (21, 13, 4, 1, 8, 20, 6),
        ] {
            let mut w = Writer::default();
            w.u32(query);
            w.0.extend([1, initial]);
            w.blob(&vec![0; cursor_size]).unwrap();
            w.u32(maximum);
            for _ in 0..count {
                w.i32(0);
            }
            w.0.push(0);
            let request = Request {
                service,
                task,
                arguments: &w.0,
            };
            assert!(Search::parse(&request).is_err());
        }
        let mut ordinary = bytes(&fixture()["requests"]["ordinary"]);
        ordinary.push(0);
        assert!(Search::parse(&Request::parse(&ordinary).unwrap()).is_err());
    }

    #[test]
    fn browser_codec_keeps_game_allocation_search_separate() {
        let wire = bytes(&fixture()["requests"]["ordinary"]);
        let request = Request::parse(&wire).unwrap();
        assert!(crate::room::description::Search::parse(request.arguments).is_err());
        let mut w = Writer::default();
        w.u32(1);
        w.u32(0);
        w.u32(10);
        w.i32(-688906341);
        w.0.push(0);
        assert!(crate::room::description::Search::parse(&w.0).is_ok());
        assert!(
            Search::parse(&Request {
                service: 21,
                task: 5,
                arguments: &w.0,
            })
            .is_err()
        );
    }

    #[test]
    fn channel_join_uses_channel_identity_and_requires_both_uint64_fields() {
        let wire = bytes(&fixture()["requests"]["channel_join"]);
        let request = Request::parse(&wire).unwrap();
        assert_eq!((request.service, request.task), (11, 11));
        assert_eq!(
            ChannelJoin::parse(request.arguments).unwrap(),
            ChannelJoin {
                channel_id: 123,
                credential: 0,
            }
        );
        for end in 0..request.arguments.len() {
            assert!(ChannelJoin::parse(&request.arguments[..end]).is_err());
        }
        let mut extra = request.arguments.to_vec();
        extra.push(0);
        assert!(ChannelJoin::parse(&extra).is_err());
    }

    #[test]
    fn listing_matches_native_result_with_real_owner_address_and_population() {
        let f = fixture();
        let descriptor = bytes(&f["listing"]["create_arguments"]);
        let expected = bytes(&f["listing"]["record"]);
        let d = Description::parse(&descriptor, 42).unwrap();
        let data = d.listing_record(&[1, 2, 3, 4, 5, 6, 7, 8], 3).unwrap();
        assert_eq!(data, expected);
        assert_eq!(d.channel_id, 123);
        assert_eq!(d.owner_name, "Lab host");
        assert_eq!(d.encoded, descriptor);
    }
}
