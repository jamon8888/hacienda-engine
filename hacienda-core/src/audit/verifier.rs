use crate::audit::AuditChain;

pub fn verify_chain(chain: &crate::audit::AuditChain) -> Result<(), String> {
    chain.verify()
}
