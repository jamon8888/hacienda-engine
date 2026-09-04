# Vault Policy for ExternalSecrets Operator - Staging
# Allows reading hacienda secrets for staging environment only

path "secret/data/hacienda/staging/*" {
  capabilities = ["read", "list"]
}

path "secret/metadata/hacienda/staging/*" {
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
