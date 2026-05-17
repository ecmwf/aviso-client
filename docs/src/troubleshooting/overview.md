# Troubleshooting

This page collects diagnostics for problems users hit in real deployments. It grows alongside features.

## Reporting a problem

Every server response carries an `X-Request-ID` header; the same UUID is embedded in the first SSE event payload's `request_id` field and in every error/control payload. Quote it when filing issues against `aviso-server` or `aviso-client`.
