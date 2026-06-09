//! Anonymous image upload via the official Imgur API.
//!
//! Imgur's documented anonymous flow: POST the image to `/3/image` with an
//! `Authorization: Client-ID <id>` header. No user account or OAuth needed.
//! Register a free Client-ID at https://api.imgur.com/oauth2/addclient
//! (choose "anonymous usage without user authorization").

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

const ENDPOINT: &str = "https://api.imgur.com/3/image";

#[derive(Deserialize)]
struct ImgurResp {
    success: bool,
    status: u32,
    data: ImgurData,
}

#[derive(Deserialize, Default)]
struct ImgurData {
    link: Option<String>,
    deletehash: Option<String>,
    error: Option<serde_json::Value>,
}

/// Result of a successful upload.
pub struct Uploaded {
    pub link: String,
    /// Anonymous delete token — keep it if you want to be able to remove the image later.
    pub deletehash: Option<String>,
}

/// Upload PNG bytes anonymously. `client_id` is your Imgur application Client-ID.
pub fn upload_png(client_id: &str, png: Vec<u8>) -> Result<Uploaded> {
    let form = reqwest::blocking::multipart::Form::new()
        .part(
            "image",
            reqwest::blocking::multipart::Part::bytes(png)
                .file_name("linkshot.png")
                .mime_str("image/png")?,
        )
        .text("type", "file");

    let client = reqwest::blocking::Client::builder()
        .user_agent("linkshot/0.1")
        .build()?;

    let resp = client
        .post(ENDPOINT)
        .header("Authorization", format!("Client-ID {}", client_id))
        .multipart(form)
        .send()
        .context("request to Imgur failed (check network/Client-ID)")?;

    let http_status = resp.status();
    let body = resp.text().unwrap_or_default();

    let parsed: ImgurResp = serde_json::from_str(&body).with_context(|| {
        format!(
            "unexpected Imgur response (HTTP {}): {}",
            http_status,
            body.chars().take(300).collect::<String>()
        )
    })?;

    if !parsed.success || parsed.status != 200 {
        let detail = parsed
            .data
            .error
            .map(|e| e.to_string())
            .unwrap_or_else(|| "unknown error".into());
        return Err(anyhow!("Imgur rejected the upload: {}", detail));
    }

    let link = parsed
        .data
        .link
        .ok_or_else(|| anyhow!("Imgur response missing image link"))?;

    Ok(Uploaded {
        link,
        deletehash: parsed.data.deletehash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_success_response() {
        let body = r#"{"data":{"id":"abc123","link":"https://i.imgur.com/abc123.png",
            "deletehash":"XyZdelete"},"success":true,"status":200}"#;
        let r: ImgurResp = serde_json::from_str(body).unwrap();
        assert!(r.success && r.status == 200);
        assert_eq!(r.data.link.as_deref(), Some("https://i.imgur.com/abc123.png"));
        assert_eq!(r.data.deletehash.as_deref(), Some("XyZdelete"));
    }

    #[test]
    fn parses_error_response() {
        let body = r#"{"data":{"error":"Invalid client_id"},"success":false,"status":403}"#;
        let r: ImgurResp = serde_json::from_str(body).unwrap();
        assert!(!r.success);
        assert!(r.data.link.is_none());
        assert!(r.data.error.is_some());
    }
}
