use crate::{
    google::LogSeverity,
    serializers::{SerializableContext, SerializableSpan, SourceLocation},
    writer::WriteAdaptor,
};
#[cfg(feature = "opentelemetry")]
use opentelemetry::trace::TraceContextExt;
use serde::ser::{SerializeMap, Serializer as _};
use std::fmt;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tracing_core::{field::Visit, Event, Subscriber};
use tracing_subscriber::{
    fmt::{
        format::{self, JsonFields},
        FmtContext, FormatEvent,
    },
    registry::LookupSpan,
};

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)]
    Formatting(#[from] fmt::Error),
    #[error("JSON serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("Time formatting error: {0}")]
    Time(#[from] time::error::Format),
}

impl From<Error> for fmt::Error {
    fn from(_: Error) -> Self {
        Self
    }
}

/// Tracing Event formatter for Stackdriver layers
pub struct EventFormatter {
    pub(crate) include_source_location: bool,
    #[cfg(feature = "opentelemetry")]
    pub(crate) cloud_trace_configuration: Option<crate::CloudTraceConfiguration>,
}

// Helper struct to capture event fields
struct EventFieldVisitor<'a>(serde_json::Map<String, serde_json::Value>, &'a JsonFields);

impl Visit for EventFieldVisitor<'_> {
    fn record_f64(&mut self, field: &tracing_core::Field, value: f64) {
        self.0.insert(
            field.name().to_string(),
            serde_json::Value::Number(serde_json::Number::from_f64(value).unwrap_or_else(|| {
                // tracing::debug!(target: "tracing_stackdriver::event_formatter", "f64 is not finite, using 0.0 instead");
                serde_json::Number::from(0)
            })),
        );
    }

    fn record_i64(&mut self, field: &tracing_core::Field, value: i64) {
        self.0.insert(
            field.name().to_string(),
            serde_json::Value::Number(value.into()),
        );
    }

    fn record_u64(&mut self, field: &tracing_core::Field, value: u64) {
        self.0.insert(
            field.name().to_string(),
            serde_json::Value::Number(value.into()),
        );
    }

    fn record_bool(&mut self, field: &tracing_core::Field, value: bool) {
        self.0
            .insert(field.name().to_string(), serde_json::Value::Bool(value));
    }

    fn record_str(&mut self, field: &tracing_core::Field, value: &str) {
        self.0.insert(
            field.name().to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }

    fn record_error(
        &mut self,
        field: &tracing_core::Field,
        value: &(dyn std::error::Error + 'static),
    ) {
        self.0.insert(
            field.name().to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }

    fn record_debug(&mut self, field: &tracing_core::Field, value: &dyn fmt::Debug) {
        match field.name() {
            // Skip fields that are actually log metadata that have already been handled
            name if name.starts_with("log.") => (),
            name if name.starts_with("event.") => (),
            "message" => {
                self.0.insert(
                    "message".to_string(), // Use "message" as the key for the message field
                    serde_json::Value::String(format!("{:?}", value)),
                );
            }
            _ => {
                self.0.insert(
                    field.name().to_string(),
                    serde_json::Value::String(format!("{:?}", value)),
                );
            }
        }
    }
}

impl EventFormatter {
    /// Internal event formatting for a given serializer
    fn format_event<S>(
        &self,
        context: &FmtContext<S, JsonFields>,
        mut serializer: serde_json::Serializer<WriteAdaptor>,
        event: &Event,
    ) -> Result<(), Error>
    where
        S: Subscriber + for<'span> LookupSpan<'span>,
    {
        let time = OffsetDateTime::now_utc().format(&Rfc3339)?;
        let meta = event.metadata();

        // attempt to resolve the explicit parent first. If that fails, fall back to the
        // current span for backwards compatibility.
        let span = event
            .parent()
            .and_then(|id| context.span(id))
            .or_else(|| context.lookup_current());

        // Extract event fields first
        let mut visitor = EventFieldVisitor(serde_json::Map::new(), context.field_format());
        event.record(&mut visitor);

        // Check if there's a custom severity in the fields, otherwise use the log level
        let severity = visitor
            .0
            .remove("severity")
            .map(LogSeverity::from)
            .unwrap_or_else(|| LogSeverity::from(meta.level()));

        let mut map = serializer.serialize_map(None)?;

        map.serialize_entry("severity", &severity)?;
        map.serialize_entry("time", &time)?;
        map.serialize_entry("target", meta.target())?;

        // Process fields with special handling for http_request, labels, and insert_id
        let mut http_request = std::collections::BTreeMap::new();
        let mut labels = std::collections::BTreeMap::new();

        for (key, value) in visitor.0 {
            let mut key_segments = key.splitn(2, '.');

            match (key_segments.next(), key_segments.next()) {
                (Some("http_request"), Some(request_key)) => {
                    use inflector::Inflector;
                    http_request.insert(request_key.to_camel_case(), value);
                }
                (Some("labels"), Some(label_key)) => {
                    use inflector::Inflector;
                    let value = match value {
                        serde_json::Value::String(value) => value,
                        _ => value.to_string(),
                    };
                    labels.insert(label_key.to_camel_case(), value);
                }
                (Some("insert_id"), None) => {
                    let value = match value {
                        serde_json::Value::String(value) => value,
                        _ => value.to_string(),
                    };
                    map.serialize_entry("logging.googleapis.com/insertId", &value)?;
                }
                (Some("message"), None) => {
                    map.serialize_entry("message", &value)?;
                }
                (Some(key), None) => {
                    use inflector::Inflector;
                    map.serialize_entry(&key.to_camel_case(), &value)?;
                }
                _ => {
                    use inflector::Inflector;
                    map.serialize_entry(&key.to_camel_case(), &value)?;
                }
            }
        }

        if !http_request.is_empty() {
            map.serialize_entry("httpRequest", &http_request)?;
        }

        if !labels.is_empty() {
            map.serialize_entry("logging.googleapis.com/labels", &labels)?;
        }

        if self.include_source_location {
            if let Some(file) = meta.file() {
                map.serialize_entry(
                    "logging.googleapis.com/sourceLocation",
                    &SourceLocation {
                        file,
                        line: meta.line(),
                    },
                )?;
            }
        }

        let spans_value = serde_json::to_value(SerializableContext::new(context))
            .map_err(Error::Serialization)?;
        let spans_array = spans_value.as_array();

        if let Some(span_ref) = span.as_ref() {
            map.serialize_entry("span", &SerializableSpan::new(&span_ref))?;
        } else if let Some(spans) = spans_array {
            if let Some(last_span) = spans.last() {
                map.serialize_entry("span", last_span)?;
            }
        }

        if spans_array.map_or(false, |arr| !arr.is_empty()) {
            map.serialize_entry("spans", &spans_value)?;
        }

        #[cfg(feature = "opentelemetry")]
        if let (Some(span_ref), Some(config)) =
            (span.as_ref(), self.cloud_trace_configuration.as_ref())
        {
            // Access OtelData to get the OpenTelemetry span information
            // that was created by tracing_opentelemetry for this tracing span
            if let Some(otel_data) = span_ref
                .extensions()
                .get::<tracing_opentelemetry::OtelData>()
            {
                let mut otel_trace_id: Option<String> = None;
                let mut otel_span_id: Option<String> = None;
                let mut otel_is_sampled: Option<bool> = None;

                // Get trace ID and sampling from the parent context
                // (trace IDs are propagated, not generated per span)
                let parent_cx = &otel_data.parent_cx;
                if parent_cx.has_active_span() {
                    let parent_span = parent_cx.span();
                    let parent_span_context = parent_span.span_context();

                    otel_trace_id = Some(format!(
                        "projects/{}/traces/{}",
                        config.project_id,
                        parent_span_context.trace_id()
                    ));
                    otel_is_sampled = Some(parent_span_context.is_sampled());
                }

                // Get span ID from builder (the span being created for this tracing span)
                // If builder.span_id is None, it means the span hasn't been started yet,
                // so we fall back to using Context::current() to get the active span
                if let Some(span_id) = otel_data.builder.span_id {
                    otel_span_id = Some(span_id.to_string());
                } else {
                    // Fallback: get the current active OpenTelemetry span
                    let current_cx = opentelemetry::Context::current();
                    if current_cx.has_active_span() {
                        let current_span = current_cx.span();
                        let current_span_context = current_span.span_context();
                        // Only use if it's not a remote span (it's a local span we created)
                        if !current_span_context.is_remote() {
                            otel_span_id = Some(current_span_context.span_id().to_string());
                        }
                    }
                }

                // Write the Cloud Trace fields
                if let Some(trace_id) = otel_trace_id {
                    map.serialize_entry("logging.googleapis.com/trace", &trace_id)?;
                }
                if let Some(span_id) = otel_span_id {
                    map.serialize_entry("logging.googleapis.com/spanId", &span_id)?;
                }
                if let Some(true) = otel_is_sampled {
                    map.serialize_entry("logging.googleapis.com/trace_sampled", &true)?;
                }
            }
        }

        map.end()?;
        Ok(())
    }
}

impl<S> FormatEvent<S, JsonFields> for EventFormatter
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    fn format_event(
        &self,
        context: &FmtContext<S, JsonFields>,
        mut writer: format::Writer,
        event: &Event,
    ) -> fmt::Result
    where
        S: Subscriber + for<'span> LookupSpan<'span>,
    {
        let serializer = serde_json::Serializer::new(WriteAdaptor::new(&mut writer));
        self.format_event(context, serializer, event)?;
        writeln!(writer)
    }
}

impl Default for EventFormatter {
    fn default() -> Self {
        Self {
            include_source_location: true,
            #[cfg(feature = "opentelemetry")]
            cloud_trace_configuration: None,
        }
    }
}
