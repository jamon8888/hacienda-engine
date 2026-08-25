# GKE Cluster Module

resource "google_container_cluster" "hacienda" {
  name     = "hacienda-${var.environment}"
  location = var.region

  # Remove default node pool
  remove_default_node_pool = true
  initial_node_count       = 1

  # Networking
  network    = var.network
  subnetwork = var.subnetwork

  # Workload Identity
  workload_identity_config {
    workload_pool = "${var.project_id}.svc.id.goog"
  }

  # Security
  master_auth {
    client_certificate_config {
      issue_client_certificate = false
    }
  }

  # Private cluster
  private_cluster_config {
    enable_private_nodes    = true
    enable_private_endpoint = false
    master_ipv4_cidr_block  = "172.16.0.0/28"
  }

  # Binary Authorization
  binary_authorization {
    evaluation_mode = "PROJECT_SINGLETON_POLICY_ENFORCE"
  }

  # Shielded nodes
  shielded_nodes {
    enable = true
  }

  # Release channel
  release_channel {
    channel = "REGULAR"
  }

  # Logging & Monitoring
  logging_config {
    component_config {
      enable_components = ["SYSTEM_COMPONENTS", "WORKLOADS"]
    }
  }

  monitoring_config {
    component_config {
      enable_components = ["SYSTEM_COMPONENTS", "WORKLOADS", "APISERVER", "SCHEDULER", "CONTROLLER_MANAGER"]
    }
  }

  # Maintenance window
  maintenance_policy {
    daily_maintenance_window {
      start_time = "04:00"
    }
  }

  # Labels
  labels = {
    environment = var.environment
    managed_by  = "terraform"
  }
}

# Node Pool - General
resource "google_container_node_pool" "general" {
  name       = "general-${var.environment}"
  cluster    = google_container_cluster.hacienda.name
  location   = var.region
  project    = var.project_id

  node_count = var.environment == "prod" ? 3 : 1

  autoscaling {
    min_node_count = var.environment == "prod" ? 3 : 1
    max_node_count = var.environment == "prod" ? 20 : 5
  }

  management {
    auto_repair  = true
    auto_upgrade = true
  }

  node_config {
    machine_type = var.environment == "prod" ? "e2-standard-4" : "e2-standard-2"
    disk_size_gb = 100
    disk_type    = "pd-ssd"
    oauth_scopes = [
      "https://www.googleapis.com/auth/cloud-platform"
    ]
    labels = {
      node-pool = "general"
    }
    workload_metadata_config {
      mode = "GKE_METADATA"
    }
    shielded_instance_config {
      enable_secure_boot          = true
      enable_integrity_monitoring = true
    }
  }

  # Upgrade settings
  upgrade_settings {
    max_surge       = 1
    max_unavailable = 0
  }
}

# Node Pool - Memory Optimized (for hacienda-api)
resource "google_container_node_pool" "memory" {
  name       = "memory-${var.environment}"
  cluster    = google_container_cluster.hacienda.name
  location   = var.region
  project    = var.project_id

  node_count = var.environment == "prod" ? 2 : 0

  autoscaling {
    min_node_count = var.environment == "prod" ? 2 : 0
    max_node_count = var.environment == "prod" ? 10 : 0
  }

  management {
    auto_repair  = true
    auto_upgrade = true
  }

  node_config {
    machine_type = "e2-highmem-4"
    disk_size_gb = 100
    disk_type    = "pd-ssd"
    oauth_scopes = [
      "https://www.googleapis.com/auth/cloud-platform"
    ]
    labels = {
      node-pool = "memory"
      workload  = "hacienda-api"
    }
    taint {
      key    = "workload"
      value  = "hacienda-api"
      effect = "NO_SCHEDULE"
    }
    workload_metadata_config {
      mode = "GKE_METADATA"
    }
  }
}

output "cluster_name" {
  value = google_container_cluster.hacienda.name
}

output "cluster_endpoint" {
  value = google_container_cluster.hacienda.endpoint
}
