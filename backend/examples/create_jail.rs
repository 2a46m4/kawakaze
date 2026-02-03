use kawakaze_backend::jail::Jail;

fn main() {
    println!("Creating a FreeBSD jail...");

    // Create a jail
    let mut jail = match Jail::create("test_jail")
        .unwrap()
        .with_path("/tmp/test_jail_root")
    {
        Ok(j) => j,
        Err(e) => {
            eprintln!("Failed to configure jail: {}", e);
            std::process::exit(1);
        }
    };

    println!("Jail configured:");
    println!("  Name: {}", jail.name());
    println!("  State: {:?}", jail.state());

    // Try to start the jail (requires root)
    match jail.start() {
        Ok(()) => {
            println!("Jail started successfully!");
            println!("  JID: {}", jail.jid());
            println!("  State: {:?}", jail.state());

            // Stop the jail
            match jail.stop() {
                Ok(()) => {
                    println!("Jail stopped successfully!");
                    println!("  State: {:?}", jail.state());
                }
                Err(e) => eprintln!("Failed to stop jail: {}", e),
            }
        }
        Err(e) => {
            eprintln!("Failed to start jail: {}", e);
            eprintln!("Note: Jail creation requires root privileges");
        }
    }
}
