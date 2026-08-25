# Vault Policy for ExternalSecrets Operator
# Allows reading all hacienda secrets across environments

path "secret/data/hacienda/*" {
  capabilities = ["read", "list"]
}

path "secret/metadata/hacienda/*" {
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
