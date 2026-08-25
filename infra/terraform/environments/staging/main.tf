# Staging Environment Configuration

module "gke" {
  source = "../../modules/gke"

  project_id = var.project_id
  region     = var.region
  environment = "staging"
  network    = var.network
  subnetwork = var.subnetwork
}

module "cloudsql" {
  source = "../../modules/cloudsql"

  project_id = var.project_id
  region     = var.region
  environment = "staging"
  network    = var.network
}

module "redis" {
  source = "../../modules/redis"

  project_id = var.project_id
  region     = var.region
  environment = "staging"
  network    = var.network
}

module "s3" {
  source = "../../modules/s3"

  project_id = var.project_id
  region     = var.region
  environment = "staging"
  network    = var.network
  domain     = var.domain
}
