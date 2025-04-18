
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
            spin build || exit
            set WASM_FILE (echo $app | tr '-' '_')
            if test -e Cargo.toml
              cp target/wasm32-wasip1/release/$WASM_FILE.wasm ../../wasm/
            else
              cp $WASM_FILE.wasm ../../wasm/
            end
            cd ..
        end
        printf "#%.0s" (seq 80)
        echo
        ;;
    case '*'
        printf "#%.0s" (seq 80)
        echo
        cd apps/{{app}}
        spin build || exit
        set WASM_FILE (echo "{{app}}" | tr '-' '_')
        cp target/wasm32-wasip1/release/app.wasm ../../wasm/$WASM_FILE.wasm
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
  spin up --from spin.dev.toml --runtime-config-file runtime-config.dev.toml

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

