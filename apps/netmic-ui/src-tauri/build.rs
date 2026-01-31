use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    tauri_build::build();
    build_netmic_server();
    build_netmic_client();
}

fn build_netmic_server() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let workspace_root = PathBuf::from(manifest_dir)
        .join("../../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("."));
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let target_dir = env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| workspace_root.join("target"));
    let server_target_dir = target_dir.join("netmic-server-build");

    let mut cmd = Command::new(cargo);
    cmd.current_dir(&workspace_root)
        .arg("build")
        .arg("-p")
        .arg("netmic-server")
        .arg("--bin")
        .arg("netmic-server");
    if profile == "release" {
        cmd.arg("--release");
    }
    cmd.env("CARGO_TARGET_DIR", &server_target_dir);

    let status = cmd.status();
    match status {
        Ok(status) if status.success() => {}
        Ok(status) => {
            panic!("build netmic-server failed with status: {status}");
        }
        Err(err) => {
            panic!("build netmic-server failed: {err}");
        }
    }

    let bin_name = if cfg!(windows) {
        "netmic-server.exe"
    } else {
        "netmic-server"
    };
    let built_server = server_target_dir.join(&profile).join(bin_name);
    if !built_server.exists() {
        panic!("netmic-server binary not found at {}", built_server.display());
    }

    let target_profile_dir = target_dir.join(&profile);
    if let Err(err) = fs::create_dir_all(&target_profile_dir) {
        panic!(
            "failed to create target dir {}: {err}",
            target_profile_dir.display()
        );
    }
    let target_server = target_profile_dir.join(bin_name);
    if let Err(err) = fs::copy(&built_server, &target_server) {
        if err.raw_os_error() == Some(26) {
            println!(
                "cargo:warning=netmic-server 正在运行，跳过覆盖 {}",
                target_server.display()
            );
            return;
        }
        panic!(
            "failed to copy netmic-server to {}: {err}",
            target_server.display()
        );
    }
}

fn build_netmic_client() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let workspace_root = PathBuf::from(manifest_dir)
        .join("../../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("."));
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let target_dir = env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| workspace_root.join("target"));
    let client_target_dir = target_dir.join("netmic-client-build");

    let mut cmd = Command::new(cargo);
    cmd.current_dir(&workspace_root)
        .arg("build")
        .arg("-p")
        .arg("netmic-client")
        .arg("--bin")
        .arg("netmic-client");
    if profile == "release" {
        cmd.arg("--release");
    }
    cmd.env("CARGO_TARGET_DIR", &client_target_dir);

    let status = cmd.status();
    match status {
        Ok(status) if status.success() => {}
        Ok(status) => {
            panic!("build netmic-client failed with status: {status}");
        }
        Err(err) => {
            panic!("build netmic-client failed: {err}");
        }
    }

    let bin_name = if cfg!(windows) {
        "netmic-client.exe"
    } else {
        "netmic-client"
    };
    let built_client = client_target_dir.join(&profile).join(bin_name);
    if !built_client.exists() {
        panic!("netmic-client binary not found at {}", built_client.display());
    }

    let target_profile_dir = target_dir.join(&profile);
    if let Err(err) = fs::create_dir_all(&target_profile_dir) {
        panic!(
            "failed to create target dir {}: {err}",
            target_profile_dir.display()
        );
    }
    let target_client = target_profile_dir.join(bin_name);
    if let Err(err) = fs::copy(&built_client, &target_client) {
        if err.raw_os_error() == Some(26) {
            println!(
                "cargo:warning=netmic-client 正在运行，跳过覆盖 {}",
                target_client.display()
            );
            return;
        }
        panic!(
            "failed to copy netmic-client to {}: {err}",
            target_client.display()
        );
    }
}
