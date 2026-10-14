use crate::backend::identity::session::Session;
use crate::backend::profile::ProfileService;
use crate::contracts::lobby::TaskResult;
use anyhow::Result;
use codol_protocol::lobby::{
    profile,
    task::{self, Request},
};
pub(crate) async fn handle(
    profiles: &ProfileService,
    request: &Request<'_>,
    session: &Session,
) -> Result<TaskResult> {
    match (request.service, request.task) {
        (8, 1 | 2) => {
            let mut reader = task::Reader::new(request.arguments);
            if request.task == 1 && reader.u64()? != session.user_id {
                return Ok(TaskResult::failure(
                    "profile_owner_mismatch",
                    task::TASK_FAILED,
                ));
            }
            reader.finish()?;
            let private = request.task == 2;
            if let Some(data) = profiles.read(session, private).await? {
                let record = if private {
                    profile::private_record(session.user_id, &data)?
                } else {
                    profile::public_record(session.user_id, &data)?
                };
                return Ok(TaskResult::success(
                    if private {
                        "private_profile"
                    } else {
                        "public_profile"
                    },
                    vec![record],
                ));
            }
            // Private profile RPC writes are not supported. A precise
            // missing result lets the target initialize its own defaults.
            Ok(TaskResult::failure("profile_missing", task::NO_PROFILE))
        }
        (8, 3) => {
            profiles.save_public(session, request.arguments).await?;
            Ok(TaskResult::success("public_profile_saved", vec![]))
        }
        _ => Ok(TaskResult::failure("unsupported_task", task::TASK_FAILED)),
    }
}
