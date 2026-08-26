# Cloud Memorystore Redis Module

resource "google_redis_instance" "hacienda" {
  name           = "hacienda-${var.environment}"
  region         = var.region
  project        = var.project_id
  tier           = var.environment == "prod" ? "STANDARD_HA" : "BASIC"
  memory_size_gb = var.environment == "prod" ? 5 : 1

  redis_version = "REDIS_7_2"

  # Network
  authorized_network = var.network

  # Persistence
  persistence_config {
    persistence_mode = "RDB"
    rdb_snapshot_period = "TWELVE_HOURS"
  }

  # Maintenance
  maintenance_policy {
    day = 7  # Sunday
    start_time {
      hours   = 4
      minutes = 0
    }
  }

  # Labels
  labels = {
    environment = var.environment
    managed_by  = "terraform"
  }

  # Redis config
  redis_configs = {
    maxmemory-policy = "allkeys-lru"
    save             = ""
    appendonly       = "yes"
  }

  # Transit encryption
  transit_encryption_mode = "SERVER_AUTHENTICATION"

  # Auth
  auth_enabled = true
}

# Store Redis password in Vault
resource "vault_generic_secret" "redis" {
  path = "secret/hacienda/${var.environment}/redis"
  data_json = jsonencode({
    url = "redis://:${google_redis_instance.hacienda.auth_string}@${google_redis_instance.hacienda.host}:6379"
  })
}

output "host" {
  value = google_redis_instance.hacienda.host
}

output "port" {
  value = google_redis_instance.hacienda.port
}

output "auth_string" {
  value     = google_redis_instance.hacienda.auth_string
  sensitive = true
}
