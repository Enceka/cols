//! Route supported task IDs to business-specific protocol handlers.
use super::handlers;
use crate::backend::commerce::service::CommerceService;
use crate::backend::files::FileService;
use crate::backend::identity::session::Session;
use crate::backend::inventory::InventoryService;
use crate::backend::profile::ProfileService;
use crate::backend::publisher::PublisherService;
use crate::backend::rooms::Rooms;
use crate::contracts::lobby::TaskResult;
use anyhow::Result;
use codol_protocol::lobby::{
    channel as lobby,
    task::{self, Request, Writer},
};
use tokio::sync::Mutex;

pub(crate) struct TaskRouter<'a> {
    pub rooms: &'a Mutex<Rooms>,
    pub profiles: &'a ProfileService,
    pub inventory: &'a InventoryService,
    pub commerce: &'a CommerceService,
    pub files: &'a FileService,
    pub publisher: &'a PublisherService,
}
impl TaskRouter<'_> {
    pub(crate) async fn handle(
        &self,
        body: &[u8],
        session: &Session,
        now: u32,
    ) -> Result<TaskResult> {
        let request = Request::parse(body)?;
        match (request.service, request.task) {
            (11 | 21, _) | (6, 14) => self.rooms.lock().await.handle(&request, session, now),
            (71, 2) => {
                let mut reader = task::Reader::new(request.arguments);
                let name = reader.string(1024)?;
                reader.finish()?;
                if name.is_empty() || name.chars().any(char::is_control) {
                    return Ok(TaskResult::failure("invalid_local_text", task::TASK_FAILED));
                }
                // Target 0xa52180 reads Bool + String. Callback 0x6aa540 names
                // the Boolean "sanitized", so unchanged local text uses false.
                let mut record = Writer::default();
                record.boolean(false);
                record.string(name);
                Ok(TaskResult::success("local_text_checked", vec![record.0]))
            }
            (71, 4) => {
                lobby::registration_build(body)?;
                Ok(TaskResult::success("registration", vec![]))
            }
            (12, 6) => {
                task::Reader::new(request.arguments).finish()?;
                let mut record = Writer::default();
                record.u32(now);
                Ok(TaskResult::success("server_time", vec![record.0]))
            }
            (10, 10 | 12 | 21) => {
                handlers::files::storage(
                    self.files,
                    self.publisher,
                    task::storage(&request)?,
                    session,
                    now,
                )
                .await
            }
            (10, 20) => handlers::files::list_publisher(self.publisher, request.arguments).await,
            (8, 1..=3) => handlers::profile::handle(self.profiles, &request, session).await,
            (80, 70 | 107 | 230 | 173 | 130 | 203 | 154) | (93, 1 | 131) => {
                handlers::commerce::handle(self.commerce, self.inventory, &request, session).await
            }
            _ => Ok(TaskResult::failure("unsupported_task", task::TASK_FAILED)),
        }
    }
}
