# Vault Policy for ExternalSecrets Operator - Production
# Allows reading hacienda secrets for production environment only

path "secret/data/hacienda/prod/*" {
  capabilities = ["read", "list"]
}

path "secret/metadata/hacienda/prod/*" {
  capabilities = ["list"]
}

# Allow reading own token info
path "auth/token/lookup-self" {
  capabilities = ["read"]
}

# Allow renewing own token
path "auth/token/renew-self" {
  capabilities = ["update"]
}
