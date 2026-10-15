//! Native pre-channel mmrsv exchange. Only a live room's owner may reply.
use super::*;
impl Rooms {
    pub(super) fn reservation(
        &mut self,
        request: &Request<'_>,
        session: &Session,
        now: u32,
    ) -> Result<TaskResult> {
        let mut r = Reader::new(request.arguments);
        let recipient = r.u64()?;
        let payload = r.blob()?;
        r.finish()?;
        ensure!(
            (12..=4096).contains(&payload.len()),
            "invalid reservation size"
        );
        ensure!(
            u32::from_le_bytes(payload[..4].try_into()?) as usize == payload.len() - 4
                && payload[4..].starts_with(b"mmrsv "),
            "unsupported instant room message"
        );
        self.reservations
            .retain(|_, (_, _, expires)| *expires > now);
        let target = if let Some(channel) = self.channels.values().find(|c| {
            c.title_id == session.title_id
                && c.owner == recipient
                && c.expires_at > now
                && c.game.is_some()
        }) {
            ensure!(
                !channel.humans.contains_key(&session.user_id),
                "reservation sender already in room"
            );
            ensure!(
                self.reservations.len() < 128
                    || self
                        .reservations
                        .contains_key(&(session.title_id, session.user_id)),
                "reservation capacity"
            );
            self.reservations.insert(
                (session.title_id, session.user_id),
                (session.into(), recipient, now + 30),
            );
            channel.humans[&recipient].identity
        } else {
            let (target, owner, _) = self
                .reservations
                .get(&(session.title_id, recipient))
                .ok_or_else(|| anyhow::anyhow!("reservation response has no pending requester"))?;
            ensure!(
                *owner == session.user_id && self.channels.values().any(|c| c.is_owner(session)),
                "reservation sender is not current owner"
            );
            *target
        };
        let mut message = Writer::default();
        message.u32(21);
        message.u64(session.user_id);
        message.string(&STANDARD.encode(session.user_id.to_string()));
        message.blob(payload)?;
        let mut result = TaskResult::success("native_room_reservation_forwarded", vec![]);
        result.push_to(session.into(), target, message.0);
        Ok(result)
    }
}
