use serde::Deserialize;
use std::collections::BTreeMap;
use time::OffsetDateTime;

#[derive(Clone, Debug, Deserialize)]
pub struct MockSourceLocation {
    pub file: String,
    pub line: String,
}

#[derive(Clone, Deserialize, Debug)]
pub struct MockDefaultEvent {
    #[serde(deserialize_with = "time::serde::rfc3339::deserialize")]
    pub time: OffsetDateTime,
    pub target: String,
    pub severity: String,
    #[serde(rename = "logging.googleapis.com/sourceLocation")]
    pub source_location: MockSourceLocation,
    #[serde(rename = "logging.googleapis.com/labels", default)]
    pub labels: BTreeMap<String, String>,
    #[serde(rename = "logging.googleapis.com/insertId", default)]
    pub insert_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MockSpan {
    #[serde(rename = "name", alias = "spanName")]
    #[allow(clippy::disallowed_names)]
    pub span_name: String,
    #[serde(default)]
    pub foo: String,
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "MockEventHelper")]
pub struct MockEventWithSpan {
    span: MockSpan,
    spans: Vec<MockSpan>,
}

#[derive(Debug, Deserialize)]
struct MockEventHelper {
    #[serde(default)]
    span: Option<MockSpan>,
    #[serde(default)]
    spans: Vec<MockSpan>,
}

impl MockEventWithSpan {
    pub fn span(&self) -> &MockSpan {
        &self.span
    }

    pub fn spans(&self) -> &[MockSpan] {
        &self.spans
    }
}

impl TryFrom<MockEventHelper> for MockEventWithSpan {
    type Error = serde_json::Error;

    fn try_from(helper: MockEventHelper) -> Result<Self, Self::Error> {
        let MockEventHelper { span, spans } = helper;

        let span = match (span, spans.last().cloned()) {
            (Some(span), _) => span,
            (None, Some(fallback)) => fallback,
            (None, None) => {
                return Err(serde_json::Error::io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "missing span",
                )))
            }
        };

        Ok(Self { span, spans })
    }
}


#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MockHttpRequest {
    pub request_method: String,
    pub latency: String,
    pub remote_ip: String,
    pub status: u16,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MockHttpEvent {
    pub http_request: MockHttpRequest,
}
