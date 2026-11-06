pub fn register(storage: &meerkat_storage::Storage, ids: &[&str]) -> meerkat_storage::Result<()> {
    let existing = storage.registered_machines()?;
    for id in ids {
        if existing.iter().any(|machine| machine.machine_id == *id) {
            continue;
        }
        storage.pair_machine(
            &meerkat_storage::MachineRecord {
                is_local: false,
                machine_id: (*id).into(),
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
