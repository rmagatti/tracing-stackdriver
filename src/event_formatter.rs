use crate::{
    google::LogSeverity,
    serializers::{SerializableContext, SerializableSpan, SourceLocation},
    writer::WriteAdaptor,
};
use serde::ser::{SerializeMap, Serializer as _};
use std::fmt;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tracing_core::{Event, Subscriber};
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
        let severity = LogSeverity::from(meta.level());

        let span = event
            .parent()
            .and_then(|id| context.span(id))
            .or_else(|| context.lookup_current());

        // FIXME: derive an accurate entry count ahead of time
        let mut map = serializer.serialize_map(None)?;

        // serialize custom fields
        map.serialize_entry("time", &time)?;
        map.serialize_entry("target", &meta.target())?;

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

        // serialize the current span and its leaves
        if let Some(span) = span {
            map.serialize_entry("span", &SerializableSpan::new(&span))?;
            map.serialize_entry("spans", &SerializableContext::new(context))?;

            #[cfg(feature = "opentelemetry")]
            if let (Some(crate::CloudTraceConfiguration { project_id }), Some(otel_data)) = (
                self.cloud_trace_configuration.as_ref(),
                span.extensions().get::<tracing_opentelemetry::OtelData>(),
            ) {
                use opentelemetry::trace::TraceContextExt;
                #[cfg(feature = "opentelemetry")]
                if let Some(config) = self.cloud_trace_configuration.as_ref() {
                    let project_id = &config.project_id;
                    // Get the OpenTelemetry context associated with the current tracing span
                    let otel_ctx = opentelemetry::Context::current();

                    // Check if this OpenTelemetry context has an active span
                    // TraceContextExt must be in scope for otel_ctx.has_active_span() and otel_ctx.span()
                    if otel_ctx.has_active_span() {
                        let otel_span_ref = otel_ctx.span();
                        let otel_span_context = otel_span_ref.span_context();

                        let span_id = otel_span_context.span_id();
                        let trace_id = otel_span_context.trace_id();
                        let is_sampled = otel_span_context.is_sampled();

                        map.serialize_entry("logging.googleapis.com/spanId", &span_id.to_string())?;
                        map.serialize_entry(
                            "logging.googleapis.com/trace",
                            &format!("projects/{}/traces/{}", project_id, trace_id),
                        )?;

                        if is_sampled {
                            map.serialize_entry("logging.googleapis.com/trace_sampled", &true)?;
                        }
                    }
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
