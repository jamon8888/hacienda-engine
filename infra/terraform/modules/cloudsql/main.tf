# Cloud SQL PostgreSQL Module

resource "google_sql_database_instance" "hacienda" {
  name             = "hacienda-${var.environment}"
  database_version = "POSTGRES_16"
  region           = var.region
  project          = var.project_id

  settings {
    tier = var.environment == "prod" ? "db-custom-4-16384" : "db-f1-micro"

    availability_type = var.environment == "prod" ? "REGIONAL" : "ZONAL"

    # Storage
    disk_size               = var.environment == "prod" ? 100 : 10
    disk_autoresize         = true
    disk_type               = "PD_SSD"
    storage_auto_resize     = true
    storage_auto_resize_limit = 500

    # Backups
    backup_configuration {
      enabled                        = true
      start_time                     = "03:00"
      point_in_time_recovery_enabled = true
      transaction_log_retention_days = 7
      backup_retention_settings {
        retained_backups = 30
        retention_unit   = "COUNT"
      }
    }

    # Maintenance
    maintenance_window {
      day  = 7  # Sunday
      hour = 4
    }

    # Database flags
    database_flags {
      name  = "max_connections"
      value = var.environment == "prod" ? "200" : "50"
    }
    database_flags {
      name  = "shared_buffers"
      value = var.environment == "prod" ? "4GB" : "128MB"
    }
    database_flags {
      name  = "effective_cache_size"
      value = var.environment == "prod" ? "12GB" : "256MB"
    }
    database_flags {
      name  = "wal_keep_size"
      value = "2GB"
    }
    database_flags {
      name  = "max_replication_slots"
      value = "10"
    }
    database_flags {
      name  = "max_wal_senders"
      value = "10"
    }

    # IP Configuration
    ip_configuration {
      ipv4_enabled            = false
      private_network         = var.network
      require_ssl             = true
      authorized_networks     = []
      allocate_ip_range       = true
      enable_private_path_for_google_cloud_services = true
    }

    # Insights
    insights_config {
      query_insights_enabled = true
      record_application_tags = true
    }

    # Labels
    user_labels = {
      environment = var.environment
      managed_by  = "terraform"
    }
  }

  # Deletion protection
  deletion_protection = var.environment == "prod"
}

# Database
resource "google_sql_database" "hacienda" {
  name     = "hacienda"
  instance = google_sql_database_instance.hacienda.name
  project  = var.project_id
}

# User
resource "google_sql_user" "hacienda" {
  name     = "hacienda"
  instance = google_sql_database_instance.hacienda.name
  project  = var.project_id
  password = random_password.hacienda.result
}

# Random password for DB user
resource "random_password" "hacienda" {
  length  = 32
  special = false
}

# Store password in Vault (requires Vault provider)
# resource "vault_generic_secret" "database" {
#   path = "secret/hacienda/${var.environment}/database"
#   data_json = jsonencode({
#     url = "postgresql://hacienda:${random_password.hacienda.result}@${google_sql_database_instance.hacienda.private_ip_address}:5432/hacienda?sslmode=require"
#   })
# }

output "instance_name" {
  value = google_sql_database_instance.hacienda.name
}

output "private_ip" {
  value     = google_sql_database_instance.hacienda.private_ip_address
  sensitive = true
}

output "connection_name" {
  value = google_sql_database_instance.hacienda.connection_name
}

output "database_password" {
  value     = random_password.hacienda.result
  sensitive = true
}
