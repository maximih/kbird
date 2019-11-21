use inotify::{Inotify, WatchMask};
use std::env::var_os;
use std::process::Command;
use sysctl::{Ctl, Sysctl};

// the path to the BIRD config
const BIRD_CFG: &'static str = "/config/bird.conf";

// the path to BIRD binary
const BIRD_BIN: &'static str = "bird";

// the path to BIRDC binary
const BIRDC_BIN: &'static str = "birdc";

// the kernel settings needed to enabled IP forwarding
const CTLNAME: &str = "net.ipv4.ip_forward";

// the env variable expected to be set to enable IP forwarding
const IP_FORWARDING: &str = "IP_FORWARDING";

fn main() {
    if var_os(IP_FORWARDING).is_some() {
        println!("checking if ip forwarding is enabled");
        match Ctl::new(CTLNAME) {
            Ok(ctl) => match ctl.value_string() {
                Ok(v) if &*v != "1" => {
                    ctl.set_value_string("1").unwrap_or_else(|e| {
                        panic!("could not enable ip forwarding: {:?}", e);
                    });
                    println!("ip forwarding was enabled successfully");
                }
                Err(e) => panic!("failed to get the current value of '{}': {:?}", CTLNAME, e),
                Ok(v) => {
                    println!("ip forwarding already has the expected value: {}", v);
                }
            },
            Err(e) => println!("failed to read {} env var: {:?}", IP_FORWARDING, e),
        }
    }

    let args = ["-f", "-c", BIRD_CFG];
    println!("starting {} with args {:?}", BIRD_BIN, args);

    let mut bird = Command::new(BIRD_BIN)
        .args(args.iter())
        .spawn()
        .expect("failed to start bird daemon");

    println!("bird is flying (pid {})", bird.id());

    // monitor the bird config for changes in another thread
    // because the main thread will block waiting for bird to exit
    std::thread::spawn(move || {
        monitor_bird_cfg(BIRD_CFG);
    });

    // run kbird as long as bird is alive
    let exit_status = bird.wait().expect("bird was not running");

    if !exit_status.success() {
        let s = "bird daemon has failed ";
        match exit_status.code() {
            Some(code) => println!("{} with status code {}", s, code),
            None => println!("{} without reporting a status code", s),
        }
        std::process::exit(1);
    }
}

// monitor the bird config file using inotify and reconfigure bird when the config was changed
fn monitor_bird_cfg(f: &str) {
    println!("watching '{}'", f);

    let mut inotify = Inotify::init().expect("failed to initialize inotify");

    let mut buffer = [0u8; 4096];

    let birdc_args = ["configure".to_string(), format!("\"{}\"", f)];

    loop {
        // add the watch inside the loop to avoid issues where
        // inotify reports only the first change
        inotify
            .add_watch(f, WatchMask::MODIFY)
            .expect("failed to add file watch");

        let _ = inotify
            .read_events_blocking(&mut buffer)
            .expect("failed to read inotify events");

        // update the running config when inotify received an event (the thread was unblocked)
        println!("{} change detected, updating bird running config", f);

        match Command::new(BIRDC_BIN).args(birdc_args.iter()).spawn() {
            Ok(mut c) => {
                // wait for the command to complete to avoid defunct processes
                if let Err(e) = c.wait() {
                    println!("unexpected error when updating bird config: {}", e);
                }
            }
            Err(e) => println!("failed to trigger the config update: {}", e),
        }
    }
}
