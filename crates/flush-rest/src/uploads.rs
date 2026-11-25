//! Attachment uploads, following `@fluxerjs/core` 3.3.0.
//!
//! Two paths exist:
//! - multipart: `payload_json` + `files[n]` parts on the message route
//!   ([`RestClient::send_message_with_files`]);
//! - upload plan: `POST /channels/{id}/attachments` returns presigned URLs,
//!   the bytes go straight to storage, multipart plans are finished with
//!   `POST /channels/{id}/attachments/complete`, and the message then
//!   references each `upload_filename` ([`RestClient::upload_attachments`]).

use serde::{Deserialize, Serialize};
use serde_json::json;

use flush_core::{AttachmentRef, Message, MessageSend, Snowflake};

use crate::client::RestClient;
use crate::error::RestError;
use crate::routes::Routes;

const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";

/// A file to attach to a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileUpload {
    pub filename: String,
    pub data: Vec<u8>,
    pub content_type: Option<String>,
    pub description: Option<String>,
}

impl FileUpload {
    pub fn new(filename: impl Into<String>, data: impl Into<Vec<u8>>) -> Self {
        Self {
            filename: filename.into(),
            data: data.into(),
            content_type: None,
            description: None,
        }
    }

    pub fn content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    fn mime(&self) -> &str {
        self.content_type.as_deref().unwrap_or(DEFAULT_CONTENT_TYPE)
    }
}

/// One entry of the upload plan request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentUploadRequest {
    pub id: u64,
    pub filename: String,
    pub file_size: u64,
    pub content_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentUploadPart {
    pub part_number: u32,
    pub upload_url: String,
}

/// One entry of the upload plan response. `upload_mode` is `singlepart`
/// (with `upload_url`) or `multipart` (with `upload_id`, `part_size`, `parts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentUploadPlan {
    pub id: u64,
    #[serde(default)]
    pub filename: String,
    pub upload_filename: String,
    #[serde(default)]
    pub file_size: u64,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub upload_mode: String,
    #[serde(default)]
    pub upload_url: Option<String>,
    #[serde(default)]
    pub upload_id: Option<String>,
    #[serde(default)]
    pub part_size: Option<u64>,
    #[serde(default, deserialize_with = "flush_core::wire::null_vec")]
    pub parts: Vec<AttachmentUploadPart>,
}

impl AttachmentUploadPlan {
    pub fn is_multipart(&self) -> bool {
        self.upload_mode == "multipart"
    }

    /// Entry for `MessageSend::attachments` once the bytes are uploaded.
    pub fn to_attachment_ref(&self) -> AttachmentRef {
        AttachmentRef {
            id: self.id,
            filename: self.filename.clone(),
            upload_filename: Some(self.upload_filename.clone()),
            file_size: Some(self.file_size),
            content_type: Some(self.content_type.clone()),
            ..Default::default()
        }
    }
}

/// One finished multipart upload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentUploadComplete {
    pub upload_filename: String,
    pub upload_id: String,
}

#[derive(Deserialize)]
struct PlanResponse {
    #[serde(default, deserialize_with = "flush_core::wire::null_vec")]
    attachments: Vec<AttachmentUploadPlan>,
}

#[derive(Deserialize)]
struct CompleteResponse {
    #[serde(default, deserialize_with = "flush_core::wire::null_vec")]
    uploads: Vec<CompleteResult>,
}

#[derive(Deserialize)]
struct CompleteResult {
    upload_filename: String,
}

impl RestClient {
    /// `POST /channels/{id}/attachments`: asks for presigned upload URLs.
    pub async fn request_attachment_uploads(
        &self,
        channel: Snowflake,
        attachments: &[AttachmentUploadRequest],
    ) -> Result<Vec<AttachmentUploadPlan>, RestError> {
        let res: PlanResponse = self
            .request(
                "POST",
                &Routes::channel_attachments(channel),
                Some(json!({ "attachments": attachments })),
                None,
                None,
            )
            .await?;
        Ok(res.attachments)
    }

    /// `POST /channels/{id}/attachments/complete`: finishes multipart
    /// uploads. Returns the completed `upload_filename`s.
    pub async fn complete_attachment_uploads(
        &self,
        channel: Snowflake,
        uploads: &[AttachmentUploadComplete],
    ) -> Result<Vec<String>, RestError> {
        let res: CompleteResponse = self
            .request(
                "POST",
                &Routes::channel_attachments_complete(channel),
                Some(json!({ "uploads": uploads })),
                None,
                None,
            )
            .await?;
        Ok(res.uploads.into_iter().map(|u| u.upload_filename).collect())
    }

    /// Runs the whole upload plan for `files`: requests the plan, sends the
    /// bytes to storage and completes multipart uploads. The result goes into
    /// `MessageSend::attachments`.
    pub async fn upload_attachments(
        &self,
        channel: Snowflake,
        files: &[FileUpload],
    ) -> Result<Vec<AttachmentRef>, RestError> {
        if files.is_empty() {
            return Ok(Vec::new());
        }
        let wanted: Vec<AttachmentUploadRequest> = files
            .iter()
            .enumerate()
            .map(|(i, f)| AttachmentUploadRequest {
                id: i as u64,
                filename: f.filename.clone(),
                file_size: f.data.len() as u64,
                content_type: f.mime().to_owned(),
            })
            .collect();
        let plan = self.request_attachment_uploads(channel, &wanted).await?;
        let mut planned: Vec<u64> = plan.iter().map(|p| p.id).collect();
        planned.sort_unstable();
        if planned != (0..files.len() as u64).collect::<Vec<_>>() {
            return Err(RestError::BadResponse(format!(
                "upload plan covers file ids {planned:?}, expected one entry for each of {} files",
                files.len()
            )));
        }
        let mut multipart = Vec::new();
        let mut refs = Vec::with_capacity(plan.len());
        for item in &plan {
            let file = files.get(item.id as usize).ok_or_else(|| {
                RestError::BadResponse(format!("upload plan names unknown file id {}", item.id))
            })?;
            if item.is_multipart() {
                let upload_id = item.upload_id.clone().ok_or_else(|| {
                    RestError::BadResponse("multipart plan without upload_id".into())
                })?;
                for (part, chunk) in split_parts(&file.data, item)? {
                    self.put_presigned(&part.upload_url, file.mime(), chunk.to_vec())
                        .await?;
                }
                multipart.push(AttachmentUploadComplete {
                    upload_filename: item.upload_filename.clone(),
                    upload_id,
                });
            } else {
                let url = item.upload_url.as_deref().ok_or_else(|| {
                    RestError::BadResponse("singlepart plan without upload_url".into())
                })?;
                let mime = if item.content_type.is_empty() {
                    file.mime()
                } else {
                    &item.content_type
                };
                self.put_presigned(url, mime, file.data.clone()).await?;
            }
            let mut r = item.to_attachment_ref();
            if r.filename.is_empty() {
                r.filename = file.filename.clone();
            }
            r.description = file.description.clone();
            refs.push(r);
        }
        if !multipart.is_empty() {
            self.complete_attachment_uploads(channel, &multipart)
                .await?;
        }
        Ok(refs)
    }

    /// `POST /channels/{id}/messages` as `multipart/form-data`:
    /// `payload_json` plus one `files[n]` part per file. Entries missing from
    /// `payload.attachments` are filled in from `files`.
    pub async fn send_message_with_files(
        &self,
        channel: Snowflake,
        payload: &MessageSend,
        files: &[FileUpload],
    ) -> Result<Message, RestError> {
        if files.is_empty() {
            return self.send_message(channel, payload).await;
        }
        let mut payload = payload.clone();
        for (i, f) in files.iter().enumerate() {
            if !payload.attachments.iter().any(|a| a.id == i as u64) {
                payload.attachments.push(AttachmentRef {
                    id: i as u64,
                    filename: f.filename.clone(),
                    description: f.description.clone(),
                    ..Default::default()
                });
            }
        }
        payload
            .validate()
            .map_err(|e| RestError::Config(e.to_string()))?;
        let json =
            serde_json::to_string(&payload).map_err(|e| RestError::Transport(e.to_string()))?;
        let boundary = new_boundary(files);
        let body = multipart_body(&boundary, &json, files);
        self.request_bytes(
            "POST",
            &Routes::channel_messages(channel),
            &format!("multipart/form-data; boundary={boundary}"),
            body,
            None,
        )
        .await
    }
}

/// Pairs each planned part with its slice of `data`, in part order.
pub fn split_parts<'a>(
    data: &'a [u8],
    plan: &'a AttachmentUploadPlan,
) -> Result<Vec<(&'a AttachmentUploadPart, &'a [u8])>, RestError> {
    let size = match plan.part_size {
        Some(n) if n > 0 => n as usize,
        _ => {
            return Err(RestError::BadResponse(
                "multipart plan without part_size".into(),
            ))
        }
    };
    let mut parts: Vec<&AttachmentUploadPart> = plan.parts.iter().collect();
    parts.sort_by_key(|p| p.part_number);
    let mut out = Vec::with_capacity(parts.len());
    for part in parts {
        let index = part
            .part_number
            .checked_sub(1)
            .ok_or_else(|| RestError::BadResponse("multipart part numbers start at 1".into()))?
            as usize;
        let start = index.saturating_mul(size);
        if start >= data.len() {
            return Err(RestError::BadResponse(format!(
                "part {} starts past the end of the file",
                part.part_number
            )));
        }
        let end = (start + size).min(data.len());
        out.push((part, &data[start..end]));
    }
    Ok(out)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn new_boundary(files: &[FileUpload]) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut n = 0u32;
    loop {
        let boundary = format!("----flush-{nanos:x}-{n:x}");
        if !files.iter().any(|f| contains(&f.data, boundary.as_bytes())) {
            return boundary;
        }
        n += 1;
    }
}

fn quoted(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "%22")
        .replace(['\r', '\n'], " ")
}

/// Encodes `payload_json` and `files[n]` as a `multipart/form-data` body.
pub fn multipart_body(boundary: &str, payload_json: &str, files: &[FileUpload]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"payload_json\"\r\n\
             Content-Type: application/json\r\n\r\n{payload_json}\r\n"
        )
        .as_bytes(),
    );
    for (i, f) in files.iter().enumerate() {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"files[{i}]\"; \
                 filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
                quoted(&f.filename),
                f.mime()
            )
            .as_bytes(),
        );
        body.extend_from_slice(&f.data);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}
