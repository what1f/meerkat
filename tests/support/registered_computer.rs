pub fn register(storage: &lankide_storage::Storage, ids: &[&str]) -> lankide_storage::Result<()> {
    let existing = storage.registered_computers()?;
    for id in ids {
        if existing.iter().any(|computer| computer.computer_id == *id) {
            continue;
        }
        storage.pair_computer(
            &lankide_storage::ComputerRecord {
                computer_id: (*id).into(),
                display_name: (*id).into(),
                credential_hash: format!("scenario-credential-{id}"),
                paired_at: 1,
                revoked_at: None,
                last_seen_at: None,
                os: "scenario".into(),
                version: "1".into(),
                inventory_json: "[]".into(),
            },
            None,
            600_000,
        )?;
    }
    Ok(())
}
