# La Chuoi Systemd User Services

This directory contains systemd user service files for running the La Chuoi Master and Worker.

## Installation

1.  **Build the project**:
    Ensure you have built the release binaries:
    ```bash
    cargo build --release
    ```

2.  **Copy the service files**:
    Copy the `.service` files to your systemd user configuration directory:
    ```bash
    mkdir -p ~/.config/systemd/user/
    cp systemd/*.service ~/.config/systemd/user/
    ```

3.  **Configure Environment Variables**:
    Ensure you have a `.env` file in the project root.
    If you want to use different environment files for the master and worker, update the `EnvironmentFile` path in the respective `.service` files.

    For example, for the worker, you might want to create a `.worker.env` with:
    ```bash
    LACHUOI_MASTER_WS_URL="ws://127.0.0.1:9130/ws/worker"
    LACHUOI_API_KEY="your-secret-key"
    ```

4.  **Reload systemd user daemon**:
    ```bash
    systemctl --user daemon-reload
    ```

## Usage

### Start the Master
```bash
systemctl --user start lachuoi
```

### Start the Worker
```bash
systemctl --user start lachuoi-worker
```

### Enable services on startup
```bash
systemctl --user enable lachuoi
systemctl --user enable lachuoi-worker
```

### Check status and logs
```bash
systemctl --user status lachuoi
journalctl --user -u lachuoi -f
```

## Note on Paths
The service files use absolute paths based on `/var/home/seungjin/Works/lachuoi-home/lachuoi`. If you move the project directory, you will need to update the `WorkingDirectory`, `ExecStart`, and `EnvironmentFile` paths in the `.service` files.
