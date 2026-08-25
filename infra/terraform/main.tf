# Root Terraform Configuration for hacienda-engine SaaS Infrastructure

terraform {
  required_version = ">= 1.5"

  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "~> 5.0"
    }
    kubernetes = {
      source  = "hashicorp/kubernetes"
      version = "~> 2.23"
    }
    helm = {
      source  = "hashicorp/helm"
      version = "~> 2.10"
    }
    vault = {
      source  = "hashicorp/vault"
      version = "~> 3.0"
    }
    external = {
      source  = "hashicorp/external"
      version = "~> 2.3"
    }
  }

  backend "gcs" {
    bucket = "hacienda-terraform-state"
    prefix = "prod"
  }
}

provider "google" {
  project = var.project_id
  region  = var.region
}

provider "google-beta" {
  project = var.project_id
  region  = var.region
}

# Kubernetes provider configured after GKE creation
provider "kubernetes" {
  host                   = google_container_cluster.hacienda.endpoint
  token                  = data.google_client_config.default.access_token
  cluster_ca_certificate = base64decode(google_container_cluster.hacienda.master_auth[0].cluster_ca_certificate)
}

provider "helm" {
  kubernetes {
    host                   = google_container_cluster.hacienda.endpoint
    token                  = data.google_client_config.default.access_token
    cluster_ca_certificate = base64decode(google_container_cluster.hacienda.master_auth[0].cluster_ca_certificate)
  }
}

provider "vault" {
  address = "https://vault.example.com"
  # Token from environment or Vault agent
}

# Data sources
data "google_client_config" "default" {}

# Variables
variable "project_id" {
  description = "GCP Project ID"
  type        = string
}

variable "region" {
  description = "GCP Region"
  type        = string
  default     = "us-central1"
}

variable "environment" {
  description = "Environment name"
  type        = string
}

variable "domain" {
  description = "Base domain for hacienda"
  type        = string
  default     = "example.com"
}

# Outputs
output "cluster_endpoint" {
  value = google_container_cluster.hacienda.endpoint
}

output "cluster_name" {
  value = google_container_cluster.hacienda.name
}

output "database_host" {
  value = google_sql_database_instance.hacienda.private_ip_address
  sensitive = true
}

output "redis_host" {
  value = google_redis_instance.hacienda.host
  sensitive = true
}
