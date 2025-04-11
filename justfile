
build app :
  #!/usr/bin/env fish
  cd {{justfile_directory()}}
  switch "{{app}}"
    case all
        cd apps
        for app in (ls -d -- */ | grep -v '^_')
            printf "#%.0s" (seq 80)
            echo
            echo "Build: $app"
            cd $app
            cargo build --target wasm32-wasip1 --release || exit
            set WASM_FILE (echo $app | tr '-' '_')
            cp target/wasm32-wasip1/release/$WASM_FILE.wasm ../../wasm/
            cd ..
        end
        printf "#%.0s" (seq 80)
        echo
        ;;
    case '*'
        printf "#%.0s" (seq 80)
        echo
        cd apps/{{app}}
        cargo build --target wasm32-wasip1 --release || exit
        set WASM_FILE (echo "{{app}}" | tr '-' '_')
        cp target/wasm32-wasip1/release/$WASM_FILE.wasm ../../wasm/
        printf "#%.0s" (seq 80)
        echo
        ;;
  end

up: 
  #!/usr/bin/env fish
  cd {{justfile_directory()}}
  for line in (cat .env | grep -v '^#' | grep -v '^[[:space:]]*$')
    set item (string split -m 1 '=' $line)
    set -gx $item[1] $item[2]
  end
  spin up --build --runtime-config-file runtime-config.dev.toml

clean:
  #!/usr/bin/env fish
  cd {{justfile_directory()}}
  for app in (ls -D ./apps/ | grep -v '^_')
    echo "Clean: $app"
    cd apps/$app
    cargo clean
    cd ../..            
  end
  trash wasm/*

