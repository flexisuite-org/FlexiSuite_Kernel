# Kernel Archiver

The `kernel-archiver` is a background service responsible for archiving `EntityHistory` and `AuditLog` records from the primary PostgreSQL database to an S3-compatible object storage.

## Purpose

To ensure long-term retention, data integrity (WORM compliance), and to keep the primary database lean, this service periodically:
1.  Queries for records where `archived_at` is `NULL`.
2.  Serializes the records to JSON.
3.  Compresses the data using Gzip.
4.  Calculates a SHA256 checksum for integrity verification.
5.  Uploads the compressed data to S3 with metadata:
    -   `worm-compliant: true`
    -   `sha256: <checksum>`
6.  Updates the database record setting `archived_at` to the current timestamp.

## Configuration

The service is configured via environment variables:

| Variable | Description | Default |
|---|---|---|
| `DATABASE_URL` | PostgreSQL connection string (Required) | - |
| `AUDIT_LOG_BUCKET` | S3 bucket name for storage | `audit-logs` |
| `AWS_REGION` | AWS Region | `us-east-1` |
| `ARCHIVER_INTERVAL` | Interval in seconds between archive cycles | `60` |
| `AWS_ACCESS_KEY_ID` | AWS Credentials (or IAM Role) | - |
| `AWS_SECRET_ACCESS_KEY` | AWS Credentials (or IAM Role) | - |

## Deployment

The archiver is designed to run as a single instance background worker (e.g., a Kubernetes Deployment with `replicas: 1` or a sidecar).

### WORM Compliance Note

This service marks objects with metadata `worm-compliant: true`. For true WORM (Write Once Read Many) protection, the target S3 bucket **MUST** be configured with Object Lock enabled in Compliance Mode.

## Development

Run locally:
```bash
cargo run -p kernel-archiver
```
