//! S3 preset registry. One row per S3-compatible service we know about;
//! entirely data — adding a new flavour is one entry, no backend code change.
//!
//! Frontend mirrors this list for the connection-modal preset picker.

#[derive(Debug, Clone, Copy)]
pub struct S3Preset {
    pub key: &'static str,
    pub label: &'static str,
    /// `None` means the user must supply an endpoint (Custom / MinIO / etc.).
    /// `Some(template)` may include `{account}` for placeholders the modal
    /// fills in before save.
    pub default_endpoint: Option<&'static str>,
    pub region_required: bool,
    pub default_path_style: bool,
}

pub const PRESETS: &[S3Preset] = &[
    S3Preset {
        key: "aws",
        label: "Amazon S3",
        default_endpoint: None, // region-templated by the modal
        region_required: true,
        default_path_style: false,
    },
    S3Preset {
        key: "r2",
        label: "Cloudflare R2",
        default_endpoint: Some("https://{account}.r2.cloudflarestorage.com"),
        region_required: false,
        default_path_style: false,
    },
    S3Preset {
        key: "minio",
        label: "MinIO (self-hosted)",
        default_endpoint: None,
        region_required: false,
        default_path_style: true,
    },
    S3Preset {
        key: "wasabi",
        label: "Wasabi",
        default_endpoint: Some("https://s3.wasabisys.com"),
        region_required: true,
        default_path_style: false,
    },
    S3Preset {
        key: "b2",
        label: "Backblaze B2",
        default_endpoint: Some("https://s3.us-west-002.backblazeb2.com"),
        region_required: true,
        default_path_style: true,
    },
    S3Preset {
        key: "gcs",
        label: "Google Cloud Storage (S3 mode)",
        default_endpoint: Some("https://storage.googleapis.com"),
        region_required: false,
        default_path_style: false,
    },
    S3Preset {
        key: "custom",
        label: "Custom S3-compatible",
        default_endpoint: None,
        region_required: false,
        default_path_style: false,
    },
];

#[allow(dead_code)]
pub fn lookup(key: &str) -> Option<&'static S3Preset> {
    PRESETS.iter().find(|p| p.key == key)
}
