# Cloud Trace Integration Fix

## Problem Summary

The previous Cloud Trace integration had a critical bug where it was attempting to extract OpenTelemetry trace and span IDs from the wrong context, resulting in:

- ❌ Missing trace correlation in Google Cloud Logging
- ❌ No "View trace details" button on logs
- ❌ Incorrect or missing span IDs
- ❌ Broken distributed tracing across services

## Root Cause

The original implementation had multiple issues:

1. **Overly Complex Logic**: The code tried to extract span information through multiple fallback paths, making it fragile and hard to debug
2. **Missing Span IDs**: When `builder.span_id` was `None` (which happens during span creation), there was no proper fallback
3. **Wrong Context Usage**: The code wasn't properly handling the case where a span is being created but hasn't been fully initialized yet

## The Solution

The fix implements a **hybrid approach** that properly extracts OpenTelemetry span information:

### Trace ID Extraction
- ✅ Gets the trace ID from the **parent context** (`otel_data.parent_cx`)
- ✅ This is correct because trace IDs are **propagated** through the entire trace tree
- ✅ Ensures all logs in a distributed trace share the same trace ID

### Span ID Extraction
- ✅ **Primary**: Uses `otel_data.builder.span_id` (the span being created for this tracing span)
- ✅ **Fallback**: Uses `Context::current()` to get the active OpenTelemetry span
- ✅ Filters out remote spans to ensure we only log local span IDs
- ✅ Each tracing span gets its own unique OpenTelemetry span ID

### Key Implementation Details

```rust
// 1. Get trace ID from parent context (propagated through the trace)
if parent_cx.has_active_span() {
    let parent_span = parent_cx.span();
    let parent_span_context = parent_span.span_context();
    otel_trace_id = Some(format!(
        "projects/{}/traces/{}",
        config.project_id,
        parent_span_context.trace_id()
    ));
}

// 2. Get span ID from builder (current span) with fallback
if let Some(span_id) = otel_data.builder.span_id {
    otel_span_id = Some(span_id.to_string());
} else {
    // Fallback to current context
    let current_cx = opentelemetry::Context::current();
    if current_cx.has_active_span() {
        let current_span = current_cx.span();
        let current_span_context = current_span.span_context();
        if !current_span_context.is_remote() {
            otel_span_id = Some(current_span_context.span_id().to_string());
        }
    }
}
```

## Usage Requirements

### 1. Layer Ordering is CRITICAL

The OpenTelemetry layer **MUST** be added **BEFORE** the Stackdriver layer:

```rust
use tracing_subscriber::layer::SubscriberExt;
use tracing_stackdriver::CloudTraceConfiguration;

let subscriber = tracing_subscriber::Registry::default()
    .with(
        // 1. OpenTelemetry layer FIRST
        tracing_opentelemetry::layer()
            .with_tracer(tracer)
    )
    .with(
        // 2. Stackdriver layer SECOND
        tracing_stackdriver::layer()
            .with_cloud_trace(CloudTraceConfiguration {
                project_id: "your-project-id".to_string(),
            })
    );

tracing::subscriber::set_global_default(subscriber)
    .expect("Failed to set subscriber");
```

### 2. Creating Spans

Always create and enter spans before logging:

```rust
use tracing::{info_span, info};

// Create a span
let span = info_span!("my_operation");
let _guard = span.enter();

// Logs within this span will have the span's OpenTelemetry span ID
info!("Operation started");
```

### 3. Distributed Tracing

For distributed tracing across services:

1. **Extract context** from incoming requests (e.g., from HTTP headers)
2. **Attach the context** before processing
3. **Create spans** as usual - they will inherit the trace ID

```rust
use opentelemetry::Context;
use tracing_opentelemetry::OpenTelemetrySpanExt;

// Extract trace context from request headers
let parent_context = extract_trace_context_from_headers(&headers);

// Create a span with the extracted context
let span = info_span!("handle_request");
span.set_parent(parent_context);
let _guard = span.enter();

// All logs now share the distributed trace ID
info!("Handling request");
```

## Verification

### Expected Log Output

Logs should now include these Cloud Trace fields:

```json
{
  "severity": "INFO",
  "time": "2025-01-16T12:34:56.789Z",
  "message": "Operation started",
  "logging.googleapis.com/trace": "projects/your-project-id/traces/67f1ab560e10b2e5e7699ae4faaae644",
  "logging.googleapis.com/spanId": "e1a343a55bf65029",
  "logging.googleapis.com/trace_sampled": true
}
```

### In Google Cloud Logging

✅ Logs should display the "View trace details" button
✅ Clicking it should navigate to Cloud Trace with the full trace hierarchy
✅ All logs from the same trace should be grouped together

## Understanding Span IDs vs Trace IDs

### Trace ID
- **Purpose**: Identifies the entire trace (all related operations)
- **Scope**: Propagated across ALL services and spans in a distributed trace
- **Example**: `projects/my-project/traces/67f1ab560e10b2e5e7699ae4faaae644`
- **Lifetime**: Remains constant throughout the entire trace

### Span ID  
- **Purpose**: Identifies a single operation/span within a trace
- **Scope**: Unique to each span/operation
- **Example**: `e1a343a55bf65029`
- **Lifetime**: Changes for each new span (parent → child → child)

### Example Trace Hierarchy

```
Trace ID: abc123 (same for all)
├─ Service A: root (span: 111)
│  └─ Service A: database_query (span: 222)
└─ Service B: process_data (span: 333)
   └─ Service B: cache_lookup (span: 444)
```

All logs from this trace will have:
- Same trace ID: `projects/my-project/traces/abc123`
- Different span IDs: 111, 222, 333, 444

## Troubleshooting

### No trace button appears on logs

**Cause**: Layer ordering is wrong
**Solution**: Ensure OpenTelemetry layer is added BEFORE Stackdriver layer

### Span IDs are all the same

**Cause**: Not creating new spans for operations
**Solution**: Use `info_span!()` or `debug_span!()` to create spans for each operation

### Missing span IDs but trace IDs are present

**Cause**: Logging outside of any span
**Solution**: Create a root span at the entry point of your service

### Trace IDs change between services

**Cause**: Not propagating context in distributed calls
**Solution**: Extract and inject trace context in HTTP headers or gRPC metadata

## Version Compatibility

This fix works with:
- `opentelemetry`: 0.29.0+
- `tracing-opentelemetry`: 0.30.0+
- `tracing-subscriber`: 0.3.18+

## Testing

Run tests to verify the fix:

```bash
cargo test --features opentelemetry
```

Both tests should pass:
- ✅ `includes_correct_cloud_trace_fields`
- ✅ `handles_nested_spans`

## Additional Resources

- [Google Cloud Trace Documentation](https://cloud.google.com/trace/docs)
- [OpenTelemetry Tracing Specification](https://opentelemetry.io/docs/concepts/signals/traces/)
- [tracing-opentelemetry Documentation](https://docs.rs/tracing-opentelemetry/)