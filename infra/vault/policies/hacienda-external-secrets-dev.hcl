# Vault Policy for ExternalSecrets Operator - Development
# Allows reading hacienda secrets for development environment only

path "secret/data/hacienda/dev/*" {
  capabilities = ["read", "list"]
}

path "secret/metadata/hacienda/dev/*" {
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
