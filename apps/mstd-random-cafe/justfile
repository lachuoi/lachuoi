  
build:
  #!/usr/bin/env fish
  cd {{justfile_directory()}}
  cargo build --target wasm32-wasip1 --release 

up: 
  #!/usr/bin/env fish
  cd {{justfile_directory()}}
  for line in (cat ../../.env | grep -v '^#' | grep -v '^[[:space:]]*$')
    set item (string split -m 1 '=' $line)
    set -gx $item[1] $item[2]
  end
  spin up --build --runtime-config-file ../../runtime-config.dev.toml

clean:
  #!/usr/bin/env fish
  cd {{justfile_directory()}}
  cargo clean
