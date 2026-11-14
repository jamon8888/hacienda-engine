# Root Terraform Configuration for hacienda-engine SaaS Infrastructure

terraform {
  required_version = ">= 1.5"

  required_providers {
    google = {
      source  = "hashicorp/google"
      # replication_spec on google_storage_transfer_job requires provider 6+.
      version = ">= 6.0, < 7.0"
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
