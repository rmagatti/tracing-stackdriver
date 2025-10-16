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
// Get span ID from builder or local parent context
// Include local parent spans (e.g., from Poem middleware) but exclude remote parent spans
// Remote parent spans are from other services and won't be in this service's trace export
let span_id = otel_data.builder.span_id.or_else(|| {
    let span_ref = otel_data.parent_cx.span();
    let span_context = span_ref.span_context();
    // Only include parent span ID if it's local (not from another service)
    if span_context.is_valid() && !span_context.is_remote() {
        Some(span_context.span_id())
    } else {
        None
    }
});
```

**Important distinction - Local vs Remote spans:**
- **Local spans**: Created by your service (middleware, instrumented functions, etc.)
  - These SHOULD be included in logs
  - They're exported by your service to Cloud Trace
  - Examples: Poem HTTP middleware spans, local tracing spans
  
- **Remote spans**: Created by other services that called yours
  - These should NOT be included in logs
  - They're not exported by your service (the calling service exports them)
  - Would cause "Missing span ID" errors in Cloud Trace
  
- **Trace ID propagation**: Always propagated (enables distributed tracing across services)
- **Span ID filtering**: Only local spans included (prevents "Missing span ID" errors)

The implementation uses `span_context.is_remote()` to distinguish between local middleware spans and remote parent spans from other services.

### 4. Critical: Layer Ordering

**⚠️ IMPORTANT:** The order of tracing layers matters significantly. The OpenTelemetry layer MUST be added before the Stackdriver layer in your subscriber chain.

**Correct order:**
```rust
let subscriber = Registry::default()
    .with(env_filter)
    .with(telemetry_layer)      // OpenTelemetry FIRST
    .with(gcp_logging_layer)     // Stackdriver SECOND
    .with(metrics_layer);
```

**Incorrect order (causes missing span IDs):**
```rust
let subscriber = Registry::default()
    .with(env_filter)
    .with(gcp_logging_layer)     // ❌ Stackdriver first
    .with(telemetry_layer)       // ❌ OpenTelemetry second
    .with(metrics_layer);
```

**Why this matters:**
- Layers process events in the order they're added
- OpenTelemetry layer populates `OtelData` (including `builder.span_id`) in span extensions
- Stackdriver layer reads `OtelData` from span extensions to format logs
- If Stackdriver runs first, `OtelData` doesn't exist yet → missing span IDs
- If OpenTelemetry runs first, `OtelData` is populated → span IDs are available

**Symptoms of incorrect ordering:**
- "Missing span ID" errors in Cloud Trace for ALL spans (including local ones)
- Logs show trace correlation but Cloud Trace can't find the spans
- `logging.googleapis.com/spanId` field is missing from log entries

### 5. Sampling Decision Detection

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

### Understanding "Missing span ID"

The "Missing span ID" message appears when logs reference a span ID that isn't exported by the current service to Cloud Trace. This commonly occurs with:

1. **Remote parent spans**: When your service receives a request with trace context from another service
2. **External middleware spans**: Spans created outside your service that propagate trace context

### The Solution

**Trace ID**: Propagate from parent context for distributed tracing
- Allows traces to span multiple services
- Enables end-to-end request tracking

**Span ID**: Only use local span IDs from `builder.span_id`
- Ensures all referenced spans are actually exported by your service
- Eliminates "Missing span ID" errors in Cloud Trace
- Your service's spans form a clean hierarchy under the distributed trace

### What You'll See

After this fix, your Cloud Trace will show:
- ✅ Complete trace hierarchy with no "Missing span ID" errors
- ✅ Only spans created and exported by your service
- ✅ Proper parent-child relationships for local spans
- ✅ Trace IDs that connect to upstream/downstream services

## Testing

All existing tests pass with OpenTelemetry 0.30:

```bash
cargo test --features opentelemetry
```

## Important Notes

1. **Project ID Must Match**: Ensure the `project_id` in `CloudTraceConfiguration` matches the GCP project where your logs are written.

2. **Spans Are Required**: You must create at least one tracing span for trace correlation to work. Logs outside of spans won't have trace fields.

3. **Trace ID Format**: The library automatically formats trace IDs as `projects/PROJECT_ID/traces/TRACE_ID` (32-character hex string).

4. **Layer Ordering**: 
   - **CRITICAL**: OpenTelemetry layer MUST come before Stackdriver layer
   - Ensures `OtelData` is populated before logs are formatted
   - Incorrect ordering causes all span IDs to be missing (not just remote ones)
   - See section "4. Critical: Layer Ordering" above for details

5. **Context Propagation**: 
   - **Trace IDs**: Extracted from builder OR parent context (enables distributed tracing)
   - **Span IDs**: Extracted from builder OR local parent context (includes local middleware, excludes remote services)
   
   This distinction is critical:
   - Trace ID propagation enables cross-service tracing
   - Span ID filtering ensures only local, exported spans are referenced
   - Uses `span_context.is_remote()` to distinguish local vs remote spans
   - Prevents referencing remote parent spans that aren't in your service's trace export

6. **Span ID Behavior**:
   - Builder span ID is checked first (current span)
   - Falls back to parent context span ID IF `!span_context.is_remote()` (local parent spans)
   - Remote parent span IDs are intentionally excluded
   - This prevents "Missing span ID" errors for cross-service calls
   - Local middleware spans (e.g., Poem HTTP spans) ARE included

## Troubleshooting

### Trace button doesn't appear in Cloud Logging

1. Verify the `opentelemetry` feature is enabled in your `Cargo.toml`
2. Check that your `CloudTraceConfiguration.project_id` matches your GCP project
3. Ensure you're creating and entering tracing spans
4. Verify the JSON output includes `logging.googleapis.com/trace` field

### Missing span ID in Cloud Trace

**First, check layer ordering!** If ALL your spans show as missing (not just remote parent spans), your layers are in the wrong order. See section "4. Critical: Layer Ordering" above.

If you see "Missing span ID" in Cloud Trace for only some spans, this is **expected behavior** for remote parent spans:

**Expected (not an error):**
- `(Missing span ID xyz123)` where `xyz123` is from an upstream service
- This happens when your service receives requests with trace context from other services
- The parent span isn't exported by your service, so Cloud Trace can't display it
- Your service's spans (including local middleware) will still appear correctly in the trace hierarchy
- Local spans like `/new-media-added` (Poem middleware) WILL have span IDs
- Only the cross-service boundary span will show as missing

**Actual problem (needs fixing):**
- Your own service's spans showing as missing
- Logs not correlating with traces at all
- No trace button appearing in Cloud Logging

To debug actual issues:
1. **CHECK LAYER ORDER FIRST** - OpenTelemetry must come before Stackdriver
2. Verify logs show `logging.googleapis.com/spanId` in the JSON output
3. Verify your OpenTelemetry middleware is creating spans correctly
4. Ensure spans are being entered (use `let _guard = span.enter()`)

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
✅ **Proper span ID handling** - only local spans referenced (no "Missing span ID" errors)  
✅ **Complete trace hierarchy** in Cloud Trace UI showing your service's spans  
✅ **Trace button** enabled in Cloud Logging for all correlated logs  
✅ **Distributed tracing support** via trace ID propagation across services  
✅ **Correct layer ordering** - documented critical setup requirement  
✅ **All tests passing** with comprehensive coverage  
✅ **Production-ready** with minimal performance overhead  

## Quick Troubleshooting Checklist

If you're seeing "Missing span ID" errors:

1. ✅ **Layer order**: Is `tracing-opentelemetry` layer added BEFORE `tracing-stackdriver`?
2. ✅ **Feature flag**: Is `features = ["opentelemetry"]` enabled for `tracing-stackdriver`?
3. ✅ **Project ID**: Does `CloudTraceConfiguration.project_id` match your GCP project?
4. ✅ **Span context**: Are you creating and entering spans with `tracing::span!`?
5. ✅ **Log output**: Do logs contain `logging.googleapis.com/spanId` field?

**Interpreting results:**
- All local spans missing (including middleware) → **Layer order is wrong**
- Only cross-service boundary span missing → **Expected for remote parent spans**
- Middleware spans like `/api/endpoint` present → **Working correctly**
