use crate::backend::files::FileService;
use crate::backend::files::valid_name;
use crate::backend::identity::session::Session;
use crate::backend::publisher::PublisherService;
use crate::contracts::lobby::TaskResult;
use anyhow::Result;
use codol_protocol::lobby::task::StorageRequest;
use codol_protocol::lobby::task::{self, Writer};
pub(crate) async fn list_publisher(
    publisher: &PublisherService,
    arguments: &[u8],
) -> Result<TaskResult> {
    let mut reader = task::Reader::new(arguments);
    let context = reader.string(16)?;
    let filter = reader.u32()?;
    let limit = reader.u16()?;
    let offset = reader.u16()?;
    reader.finish()?;
    // Only the observed unfiltered query is supported; filter/offset
    // semantics need more evidence before implementing pagination.
    if !context.is_empty() || filter != 0 || offset != 0 || limit != 32 {
        return Ok(TaskResult::failure(
            "unsupported_publisher_query",
            task::TASK_FAILED,
        ));
    }
    let mut records = Vec::new();
    for file in publisher.list(limit).await? {
        // VA 0xa5b230: this result type differs from upload FileInfo.
        let mut record = Writer::default();
        record.u32(file.size);
        record.u64(file.id);
        record.u32(file.modified);
        record.u32(file.modified);
        record.boolean(true);
        record.u64(0);
        record.string(&file.name);
        record.string(""); // Required String(cap=50), locally unset; semantics still unknown.
        records.push(record.0);
    }
    Ok(TaskResult::success("publisher_file_list", records))
}
pub(crate) async fn storage(
    files: &FileService,
    publisher: &PublisherService,
    request: StorageRequest<'_>,
    session: &Session,
    now: u32,
) -> Result<TaskResult> {
    let (context, name) = match &request {
        StorageRequest::Read { context, name, .. }
        | StorageRequest::Publisher { context, name }
        | StorageRequest::Upload { context, name, .. } => (*context, *name),
    };
    if !context.is_empty() || !valid_name(name) {
        return Ok(TaskResult::failure(
            "unsupported_storage_namespace",
            task::TASK_FAILED,
        ));
    }
    match request {
        StorageRequest::Publisher { .. } => {
            // Only operator-provided files, never generated empty config files.
            match publisher.read(name).await? {
                None => Ok(TaskResult::failure("publisher_file_missing", task::NO_FILE)),
                Some(data) => blob_result("publisher_file", &data),
            }
        }
        StorageRequest::Read {
            owner,
            account_type,
            ..
        } => {
            if owner != session.user_id || !account_type.is_empty() {
                return Ok(TaskResult::failure(
                    "storage_owner_mismatch",
                    task::TASK_FAILED,
                ));
            }
            match files.read(session, name).await? {
                None => Ok(TaskResult::failure("user_file_missing", task::NO_FILE)),
                Some(data) => blob_result("user_file", &data),
            }
        }
        StorageRequest::Upload {
            visibility,
            data,
            owner,
            account_type,
            version,
            ..
        } => {
            if (owner != 0 && owner != session.user_id) || !account_type.is_empty() || version != 0
            {
                return Ok(TaskResult::failure(
                    "unsupported_upload_parameters",
                    task::TASK_FAILED,
                ));
            }
            let record = files.upload(session, name, visibility, data, now).await?;
            // Target bdFileInfo::deserialize VA 0xa4eab0. No fileID field.
            let mut info = Writer::default();
            info.u32(data.len() as u32);
            info.u32(record.created);
            info.u32(now);
            info.boolean(visibility);
            info.u64(session.user_id);
            info.string("");
            info.string(name);
            Ok(TaskResult::success("user_file_uploaded", vec![info.0]))
        }
    }
}
fn blob_result(label: &'static str, data: &[u8]) -> Result<TaskResult> {
    let mut record = Writer::default();
    record.blob(data)?;
    Ok(TaskResult::success(label, vec![record.0]))
}
