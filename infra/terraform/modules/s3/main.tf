# S3 / GCS Bucket Module

resource "google_storage_bucket" "hacienda" {
  name          = "hacienda-${var.environment}"
  location      = var.region
  project       = var.project_id
  force_destroy = var.environment != "prod"

  # Versioning
  versioning {
    enabled = true
  }

  # Lifecycle
  lifecycle_rule {
    condition {
      age = 365
    }
    action {
      type = "Delete"
    }
  }

  lifecycle_rule {
    condition {
      num_newer_versions = 10
    }
    action {
      type = "Delete"
    }
  }

  # CORS
  cors {
    origin          = ["https://hacienda.${var.domain}", "https://hacienda-staging.${var.domain}"]
    method          = ["GET", "PUT", "POST", "DELETE", "HEAD"]
    response_header = ["*"]
    max_age_seconds = 3600
  }

  # Uniform bucket-level access
  uniform_bucket_level_access = true

  # Public access prevention
  public_access_prevention = "enforced"

  # Labels
  labels = {
    environment = var.environment
    managed_by  = "terraform"
  }
}

# Secondary bucket for cross-region replication (prod only)
resource "google_storage_bucket" "hacienda_dr" {
  count         = var.environment == "prod" ? 1 : 0
  name          = "hacienda-${var.environment}-dr"
  location      = var.dr_region
  project       = var.project_id
  force_destroy = false

  versioning {
    enabled = true
  }

  lifecycle_rule {
    condition {
      age = 365
    }
    action {
      type = "Delete"
    }
  }

  uniform_bucket_level_access = true
  public_access_prevention    = "enforced"

  labels = {
    environment = var.environment
    purpose     = "disaster-recovery"
    managed_by  = "terraform"
  }
}

# Service Account for hacienda API
resource "google_service_account" "hacienda" {
  account_id   = "hacienda-${var.environment}"
  display_name = "hacienda ${var.environment} API"
  project      = var.project_id
}

# IAM bindings
resource "google_storage_bucket_iam_member" "hacienda_writer" {
  bucket = google_storage_bucket.hacienda.name
  role   = "roles/storage.objectAdmin"
  member = "serviceAccount:${google_service_account.hacienda.email}"
}

resource "google_storage_bucket_iam_member" "hacienda_reader" {
  bucket = google_storage_bucket.hacienda.name
  role   = "roles/storage.objectViewer"
  member = "serviceAccount:${google_service_account.hacienda.email}"
}

# DR bucket IAM (prod only)
resource "google_storage_bucket_iam_member" "hacienda_dr_writer" {
  count  = var.environment == "prod" ? 1 : 0
  bucket = google_storage_bucket.hacienda_dr[0].name
  role   = "roles/storage.objectAdmin"
  member = "serviceAccount:${google_service_account.hacienda.email}"
}

# HMAC keys for S3-compatible access
resource "google_storage_hmac_key" "hacienda" {
  service_account_email = google_service_account.hacienda.email
  project               = var.project_id
}

# Store HMAC keys in Vault
# resource "vault_generic_secret" "s3" {
#   path = "secret/hacienda/${var.environment}/s3"
#   data_json = jsonencode({
#     access_key = google_storage_hmac_key.hacienda.access_id
#     secret_key = google_storage_hmac_key.hacienda.secret
#     endpoint   = "https://storage.googleapis.com"
#     region     = var.region
#     bucket     = google_storage_bucket.hacienda.name
#   })
# }

output "bucket_name" {
  value = google_storage_bucket.hacienda.name
}

output "dr_bucket_name" {
  value = var.environment == "prod" ? google_storage_bucket.hacienda_dr[0].name : ""
}

output "service_account_email" {
  value = google_service_account.hacienda.email
}

output "hmac_access_id" {
  value     = google_storage_hmac_key.hacienda.access_id
  sensitive = true
}

output "hmac_secret" {
  value     = google_storage_hmac_key.hacienda.secret
  sensitive = true
}
