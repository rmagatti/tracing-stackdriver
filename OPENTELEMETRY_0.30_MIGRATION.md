# OpenTelemetry 0.30 Migration Guide

## Overview

This document describes the changes made to support OpenTelemetry 0.30 instead of 0.31, ensuring compatibility with `poem` and other libraries that haven't yet upgraded to OpenTelemetry 0.31.

## Changes Made

### 1. Dependency Downgrades

**Cargo.toml Changes:**
- `opentelemetry`: Downgraded from `0.31.0` to `0.30.0`
- `tracing-opentelemetry`: Updated to `0.31.0` (which supports OpenTelemetry 0.30)
- `opentelemetry_sdk`: Downgraded from `0.31.0` to `0.30.0` in dev-dependencies

### 2. OtelData API Changes

The `OtelData` API changed between OpenTelemetry versions. In version 0.30, the trace and span IDs are accessed through public fields on the `builder` struct rather than through methods.

**Before (0.31+):**
```rust
if let Some(trace_id) = otel_data.trace_id() {
    // use trace_id
}
```

**After (0.30):**
```rust
if let Some(trace_id) = otel_data.builder.trace_id {
    // use trace_id
}
```

### 3. Enhanced Context Extraction

The implementation now extracts both trace ID and span ID from the parent context when they're not directly available in the builder. This ensures proper trace correlation and resolves the "Missing span ID" issue in Cloud Trace.

#### Trace ID Extraction
```rust
let trace_id = otel_data.builder.trace_id.or_else(|| {
    let span_ref = otel_data.parent_cx.span();
    let span_context = span_ref.span_context();
    if span_context.is_valid() {
        Some(span_context.trace_id())
    } else {
        None
    }
});
```

#### Span ID Extraction
```rust
let span_id = otel_data.builder.span_id.or_else(|| {
    let span_ref = otel_data.parent_cx.span();
    let span_context = span_ref.span_context();
    if span_context.is_valid() {
        Some(span_context.span_id())
    } else {
        None
    }
});
```

This dual-extraction approach ensures that:
- Root spans from external requests (e.g., HTTP middleware) have proper span IDs
- Child spans created within your application also have correct span IDs
- The full trace hierarchy is preserved in Cloud Trace

### 4. Sampling Decision Detection

The sampling decision is now checked using pattern matching on the `SamplingDecision` enum:

```rust
if let Some(sampling_result) = &otel_data.builder.sampling_result {
    otel_is_sampled = Some(matches!(
        sampling_result.decision,
        opentelemetry::trace::SamplingDecision::RecordAndSample
    ));
}
```

## Usage Example

Here's a complete example of how to set up tracing with OpenTelemetry 0.30 and Cloud Trace integration:

```rust
use tracing_stackdriver::CloudTraceConfiguration;
use tracing_subscriber::layer::SubscriberExt;
use opentelemetry::trace::TracerProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;

fn main() {
    // 1. Set up OpenTelemetry tracer (0.30)
    let tracer_provider = SdkTracerProvider::builder()
        .build();
    
    // 2. Create the OpenTelemetry tracing layer
    let opentelemetry_layer = tracing_opentelemetry::layer()
        .with_tracer(tracer_provider.tracer("my-service"));
    
    // 3. Create the Stackdriver layer with Cloud Trace configuration
    let stackdriver_layer = tracing_stackdriver::layer()
        .with_cloud_trace(CloudTraceConfiguration {
            project_id: "my-gcp-project-id".to_string(),
        });
    
    // 4. Combine layers into a subscriber
    let subscriber = tracing_subscriber::Registry::default()
        .with(opentelemetry_layer)
        .with(stackdriver_layer);
    
    // 5. Set as the global default subscriber
    tracing::subscriber::set_global_default(subscriber)
        .expect("Failed to set subscriber");
    
    // 6. Use spans to create trace context
    let root_span = tracing::info_span!("handle_request");
    let _enter = root_span.enter();
    
    // These logs will now include Cloud Trace correlation fields!
    tracing::info!("Processing request");
    
    // Nested spans work too
    let child_span = tracing::debug_span!("process_data");
    let _child_enter = child_span.enter();
    tracing::debug!("Processing data");
}
```

## Compatibility with Poem

This downgrade ensures compatibility with `poem` web framework and its OpenTelemetry integration:

```toml
[dependencies]
poem = "1.3"
poem-opentelemetry = "2.0"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["registry", "env-filter"] }
tracing-stackdriver = { version = "0.10", features = ["opentelemetry"] }
tracing-opentelemetry = "0.31"
opentelemetry = "0.30"
opentelemetry_sdk = { version = "0.30", features = ["rt-tokio"] }
```

### Poem Integration Example

```rust
use poem::{Route, Server, get, handler};
use poem::listener::TcpListener;
use poem_opentelemetry::OpenTelemetryTracingLayer;
use tracing_stackdriver::CloudTraceConfiguration;
use tracing_subscriber::layer::SubscriberExt;
use opentelemetry::trace::TracerProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;

#[handler]
async fn index() -> &'static str {
    tracing::info!("Handler called");
    "Hello, World!"
}

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    // Set up OpenTelemetry with Stackdriver
    let tracer_provider = SdkTracerProvider::builder()
        .build();
    
    let tracer = tracer_provider.tracer("poem-service");
    
    let opentelemetry_layer = tracing_opentelemetry::layer()
        .with_tracer(tracer.clone());
    
    let stackdriver_layer = tracing_stackdriver::layer()
        .with_cloud_trace(CloudTraceConfiguration {
            project_id: std::env::var("GCP_PROJECT_ID")
                .expect("GCP_PROJECT_ID must be set"),
        });
    
    let subscriber = tracing_subscriber::Registry::default()
        .with(opentelemetry_layer)
        .with(stackdriver_layer);
    
    tracing::subscriber::set_global_default(subscriber)
        .expect("Failed to set subscriber");
    
    // Set up Poem with OpenTelemetry middleware
    let app = Route::new()
        .at("/", get(index))
        .with(OpenTelemetryTracingLayer::new(tracer));
    
    Server::new(TcpListener::bind("0.0.0.0:3000"))
        .run(app)
        .await
}
```

## Cloud Logging Output

With this setup, your logs will automatically include Cloud Trace correlation fields:

```json
{
  "severity": "INFO",
  "time": "2025-10-15T17:58:08.093Z",
  "target": "my_app",
  "message": "Processing request",
  "logging.googleapis.com/trace": "projects/my-project-id/traces/7bc81f9b57612b732ee4b747113c57d3",
  "logging.googleapis.com/spanId": "57ccf9ae4e434845",
  "logging.googleapis.com/trace_sampled": false
}
```

This enables the **Trace** button in Google Cloud Logging console, allowing you to:
- Jump directly from logs to the associated trace
- View distributed traces across services
- Correlate logs with performance data
- See the complete trace hierarchy with proper span relationships

## Cloud Trace Behavior

### Before the Fix
The first trace screenshot showed a "Missing span ID" at the root level, which made it difficult to correlate the root span with logs from the entry point of your service.

### After the Fix
Now the root span properly shows its span ID (e.g., `57ccf9ae4e434845`), and all child spans are correctly nested under it. This provides a complete view of the request flow through your service.

## Testing

All existing tests pass with OpenTelemetry 0.30:

```bash
cargo test --features opentelemetry
```

## Important Notes

1. **Project ID Must Match**: Ensure the `project_id` in `CloudTraceConfiguration` matches the GCP project where your logs are written.

2. **Spans Are Required**: You must create at least one tracing span for trace correlation to work. Logs outside of spans won't have trace fields.

3. **Trace ID Format**: The library automatically formats trace IDs as `projects/PROJECT_ID/traces/TRACE_ID` (32-character hex string).

4. **Context Propagation**: The implementation now properly extracts both trace IDs and span IDs from the parent context, ensuring correct trace correlation even when these values aren't directly in the span builder. This is critical for:
   - HTTP middleware that creates root spans
   - Cross-service trace propagation
   - Correct parent-child span relationships

5. **Span ID Extraction Priority**:
   - First checks `otel_data.builder.span_id` (current span)
   - Falls back to `otel_data.parent_cx.span().span_context().span_id()` (parent context)
   - This ensures root spans from middleware have proper span IDs

## Troubleshooting

### Trace button doesn't appear in Cloud Logging

1. Verify the `opentelemetry` feature is enabled in your `Cargo.toml`
2. Check that your `CloudTraceConfiguration.project_id` matches your GCP project
3. Ensure you're creating and entering tracing spans
4. Verify the JSON output includes `logging.googleapis.com/trace` field

### Missing span ID in Cloud Trace

This should now be fixed. If you still see "Missing span ID":
1. Ensure you're using the latest version with the parent context extraction
2. Verify your OpenTelemetry middleware is creating spans correctly
3. Check that the `tracing-opentelemetry` layer is added before the `tracing-stackdriver` layer

### Version conflicts

If you encounter version conflicts:

```bash
cargo tree | grep opentelemetry
```

Make sure:
- All `opentelemetry` crates are at version `0.30.x`
- `tracing-opentelemetry` is at version `0.31.x`
- No dependencies are pulling in `opentelemetry` 0.31+

Example of correct dependency tree:
```
├── opentelemetry v0.30.0
├── opentelemetry_sdk v0.30.0
│   └── opentelemetry v0.30.0
└── tracing-opentelemetry v0.31.0
    ├── opentelemetry v0.30.0
    └── opentelemetry_sdk v0.30.0
```

### Spans not showing in trace hierarchy

Ensure your span creation follows this pattern:

```rust
// Create and enter span
let span = tracing::info_span!("operation_name");
let _guard = span.enter();

// Your code here - logs will be correlated

// Span automatically exits when _guard is dropped
```

## Future Migration Path

When poem and other dependencies upgrade to OpenTelemetry 0.31+, you can upgrade this library by:

1. Updating dependencies to use OpenTelemetry 0.31+
2. The current implementation should continue to work as the parent context extraction is a robust approach
3. Consider checking if newer OpenTelemetry versions provide better APIs for trace/span ID extraction

## Performance Considerations

The parent context extraction adds minimal overhead:
- Only executes when `otel_data.builder.trace_id` or `otel_data.builder.span_id` is `None`
- Uses efficient field access and pattern matching
- No additional allocations beyond the string formatting for the trace ID

## References

- [OpenTelemetry 0.30 Documentation](https://docs.rs/opentelemetry/0.30.0/)
- [tracing-opentelemetry Documentation](https://docs.rs/tracing-opentelemetry/0.31.0/)
- [Cloud Trace LogEntry Fields](https://cloud.google.com/logging/docs/agent/logging/configuration#special-fields)
- [Cloud Trace Overview](https://cloud.google.com/trace/docs/overview)
- [Poem OpenTelemetry Integration](https://docs.rs/poem-opentelemetry/)
- [Google Cloud Logging Structured Logs](https://cloud.google.com/logging/docs/structured-logging)

## Summary of Benefits

✅ **Full OpenTelemetry 0.30 compatibility** for use with Poem and similar frameworks  
✅ **Proper span ID extraction** fixing the "Missing span ID" issue  
✅ **Complete trace hierarchy** in Cloud Trace UI  
✅ **Trace button** enabled in Cloud Logging for all correlated logs  
✅ **Robust context propagation** using parent context fallback  
✅ **All tests passing** with comprehensive coverage  
✅ **Production-ready** with minimal performance overhead  
