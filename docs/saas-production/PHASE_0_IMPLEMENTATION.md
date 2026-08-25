# Phase 0 Implementation: Foundation (Weeks 1-2)

> **Goal**: Establish production infrastructure prerequisites
> **Duration**: 2 weeks (10 working days)
> **Team**: 2 Platform Engineers + 1 Backend Engineer

---

## Week 1: Infrastructure Provisioning

### Day 1-2: Secrets Management (Vault + ExternalSecrets)

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Provision HashiCorp Vault cluster | Platform | Vault UI accessible, unsealed, audit logging enabled |
| Configure Kubernetes auth method | Platform | `hacienda-external-secrets` role can read `secret/data/hacienda/*` |
| Install ExternalSecrets Operator | Platform | `ExternalSecret` CRD available, `ClusterSecretStore` created |
| Create secret structure in Vault | Platform | All paths from `secrets.md` created with placeholder values |
| Test ExternalSecret sync | Platform | `hacienda-secrets` K8s secret populated from Vault |

#### Commands
```bash
# 1. Deploy Vault (Helm)
helm repo add hashicorp https://helm.releases.hashicorp.com
helm install vault hashicorp/vault \
  --namespace vault --create-namespace \
  --set server.ha.enabled=true,server.ha.raft.enabled=true

# 2. Initialize and unseal
vault operator init -key-shares=5 -key-threshold=3
vault operator unseal <key1>
vault operator unseal <key2>
vault operator unseal <key3>

# 3. Enable KV v2 secrets engine
vault secrets enable -path=secret kv-v2

# 4. Create policy for external-secrets
vault policy write hacienda-external-secrets - <<EOF
path "secret/data/hacienda/*" {
  capabilities = ["read", "list"]
}
path "secret/metadata/hacienda/*" {
  capabilities = ["list"]
}
EOF

# 5. Enable Kubernetes auth
vault auth enable kubernetes
vault write auth/kubernetes/config \
  token_reviewer_jwt="$(cat /var/run/secrets/kubernetes.io/serviceaccount/token)" \
  kubernetes_host="https://$KUBERNETES_PORT_443_TCP_ADDR:443" \
  kubernetes_ca_cert=@/var/run/secrets/kubernetes.io/serviceaccount/ca.crt

# 6. Create role
vault write auth/kubernetes/role/hacienda-external-secrets \
  bound_service_account_names=external-secrets \
  bound_service_account_namespaces=external-secrets \
  policies=hacienda-external-secrets \
  ttl=24h
```

### Day 3-4: PostgreSQL Provisioning

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Provision managed PostgreSQL (Cloud SQL / RDS) | Platform | Instance running, HA enabled, backups configured |
| Create `hacienda` database and user | Platform | User with appropriate permissions |
| Configure SSL/TLS | Platform | `sslmode=required` enforced |
| Store connection string in Vault | Platform | `secret/hacienda/prod/database/url` populated |
| Test ExternalSecret sync for DB URL | Platform | K8s secret has valid `DATABASE_URL` |
| Run migrations against new DB | Backend | `sqlx migrate run` succeeds |

#### Commands
```bash
# Cloud SQL (GCP) example
gcloud sql instances create hacienda-prod \
  --database-version=POSTGRES_16 \
  --tier=db-custom-4-16384 \
  --region=us-central1 \
  --availability-type=REGIONAL \
  --enable-bin-log \
  --backup-start-time=03:00

# Create database and user
gcloud sql databases create hacienda --instance=hacienda-prod
gcloud sql users create hacienda --instance=hacienda-prod --password=<generated>

# Store in Vault
vault kv put secret/hacienda/prod/database \
  url="postgresql://hacienda:<pass>@<host>:5432/hacienda?sslmode=require"

# Run migrations (from hacienda-engine root)
DATABASE_URL="postgresql://..." sqlx migrate run
```

### Day 5: S3-Compatible Object Store

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Provision S3 bucket (or GCS / MinIO) | Platform | Bucket created with versioning, lifecycle, CORS |
| Configure IAM for hacienda access | Platform | Service account with read/write on bucket |
| Enable Cross-Region Replication (CRR) | Platform | Replication to secondary region configured |
| Store credentials in Vault | Platform | `secret/hacienda/prod/s3` with access/secret keys |
| Test presigned upload from cluster | Backend | `hacienda upload presign` works end-to-end |

#### Commands
```bash
# AWS S3 example
aws s3api create-bucket --bucket hacienda-prod --region us-east-1
aws s3api put-bucket-versioning --bucket hacienda-prod --versioning-configuration Status=Enabled
aws s3api put-bucket-lifecycle-configuration --bucket hacienda-prod --lifecycle-configuration file://lifecycle.json

# CRR (requires versioning on both)
aws s3api put-bucket-replication --bucket hacienda-prod --replication-configuration file://crr.json

# Create IAM user for hacienda
aws iam create-user --user-name hacienda-prod
aws iam put-user-policy --user-name hacienda-prod --policy-name HaciendaS3Access --policy-document file://s3-policy.json

# Store credentials
vault kv put secret/hacienda/prod/s3 \
  access_key=<access_key> \
  secret_key=<secret_key>
```

---

## Week 2: Kubernetes & GitOps

### Day 6-7: Kubernetes Cluster

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Provision GKE/EKS/AKS cluster | Platform | 3 nodes, 3 zones, workload identity |
| Install CNI (Cilium/Calico), CSI, cert-manager | Platform | All system pods healthy |
| Configure ExternalDNS | Platform | DNS records auto-created for Ingress |
| Create `hacienda-prod` namespace | Platform | Namespace with labels, resource quotas |
| Configure workload identity for Vault | Platform | Pods can authenticate to Vault |

#### Commands
```bash
# GKE example
gcloud container clusters create hacienda-prod \
  --region=us-central1 \
  --num-nodes=3 \
  --enable-autoscaling --min-nodes=3 --max-nodes=20 \
  --workload-pool=my-project.svc.id.goog \
  --addons=GcsFuseCsiDriver \
  --enable-ip-alias \
  --network=projects/my-project/global/networks/default \
  --subnetwork=projects/my-project/regions/us-central1/subnetworks/default

# Install cert-manager
kubectl apply -f https://github.com/cert-manager/cert-manager/releases/download/v1.13.0/cert-manager.yaml

# Install ExternalDNS
helm repo add external-dns https://kubernetes-sigs.github.io/external-dns/
helm install external-dns external-dns/external-dns \
  --namespace external-dns --create-namespace \
  --set provider=google \
  --set google.project=my-project
```

### Day 8: GitOps (ArgoCD)

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Install ArgoCD | Platform | ArgoCD UI accessible, admin password set |
| Create `hacienda-prod` Application | Platform | Application points to `deploy/overlays/production` |
| Configure auto-sync + prune | Platform | Changes to Git auto-deploy |
| Configure image updater | Platform | New GHCR tags auto-update Kustomization |
| Test sync with staging overlay | Platform | Staging environment deploys successfully |

#### Commands
```bash
# Install ArgoCD
kubectl create namespace argocd
kubectl apply -n argocd -f https://raw.githubusercontent.com/argoproj/argo-cd/stable/manifests/install.yaml

# Get admin password
kubectl -n argocd get secret argocd-initial-admin-secret -o jsonpath="{.data.password}" | base64 -d

# Create Application
cat <<EOF | kubectl apply -f -
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: hacienda-prod
  namespace: argocd
spec:
  project: default
  source:
    repoURL: https://github.com/jamon8888/hacienda-engine
    targetRevision: main
    path: deploy/overlays/production
  destination:
    server: https://kubernetes.default.svc
    namespace: hacienda-prod
  syncPolicy:
    automated:
      prune: true
      selfHeal: true
    syncOptions:
      - CreateNamespace=true
EOF
```

### Day 9: TLS & Ingress

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Configure Let's Encrypt ClusterIssuer | Platform | `letsencrypt-prod` ClusterIssuer ready |
| Deploy Ingress with TLS | Platform | `hacienda.example.com` serves valid cert |
| Configure security headers (HSTS, CSP) | Platform | Headers present in response |
| Configure rate limiting at ingress | Platform | 429 returned after threshold |
| Test end-to-end HTTPS | Platform | `curl https://hacienda.example.com/health` returns 200 |

#### Commands
```bash
# ClusterIssuer
cat <<EOF | kubectl apply -f -
apiVersion: cert-manager.io/v1
kind: ClusterIssuer
metadata:
  name: letsencrypt-prod
spec:
  acme:
    server: https://acme-v02.api.letsencrypt.org/directory
    email: ops@example.com
    privateKeySecretRef:
      name: letsencrypt-prod
    solvers:
      - http01:
          ingress:
            class: nginx
EOF

# Verify cert
kubectl get certificate -n hacienda-prod hacienda-tls
```

### Day 10: Validation & Gate Criteria

```bash
# All must pass before Phase 1
✅ Vault unsealed and accessible
✅ Postgres migrations successful
✅ S3 bucket accessible from cluster
✅ K8s cluster healthy (3 nodes, all ready)
✅ ArgoCD application synced
✅ TLS certificate valid
✅ Smoke test: curl https://hacienda.example.com/health
```

---

## Dependencies Between Tasks

```
Vault ──▶ ExternalSecrets ──▶ K8s Secrets
    │
    ├──▶ PostgreSQL URL
    ├──▶ S3 credentials
    ├──▶ Redis URL
    ├──▶ Pseudonym keys
    └──▶ JWT secret

PostgreSQL ──▶ Migrations

K8s Cluster ──▶ ArgoCD ──▶ Deploy manifests
    │
    ├──▶ cert-manager ──▶ TLS cert
    ├──▶ ExternalDNS ──▶ DNS records
    └──▶ Ingress ──▶ HTTPS endpoint
```

---

## Risk Mitigation

| Risk | Mitigation |
|------|------------|
| Vault unseal keys lost | Store in 1Password/physical safe; 3 of 5 required |
| PG provisioning delays | Start Day 1; use managed service for speed |
| DNS propagation | Use low TTL (60s) during setup |
| ArgoCD sync conflicts | Use `server-side apply`; test in staging first |
| Cert-manager rate limits | Use staging issuer for testing; production for final |

---

## Handoff to Phase 1

Phase 1 can begin when:

1. All Day 10 gate criteria pass
2. `hacienda-api` deploys to staging via ArgoCD
3. Staging `/health` and `/ready` endpoints return 200
4. Staging can connect to PG, Redis, S3
5. Team has runbook for common staging issues
