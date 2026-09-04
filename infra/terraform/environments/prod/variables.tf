# Production Environment Variables

variable "project_id" {
  description = "GCP Project ID"
  type        = string
  default     = "my-hacienda-project"
}

variable "region" {
  description = "Primary GCP Region"
  type        = string
  default     = "us-central1"
}

variable "dr_region" {
  description = "Disaster Recovery Region"
  type        = string
  default     = "us-east1"
}

variable "network" {
  description = "VPC Network name"
  type        = string
  default     = "default"
}

variable "subnetwork" {
  description = "Subnetwork for GKE nodes"
  type        = string
  default     = "default"
}

variable "domain" {
  description = "Base domain for hacienda"
  type        = string
  default     = "example.com"
}
