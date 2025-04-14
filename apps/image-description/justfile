build:
  #!/usr/bin/env fish
  spin build
  
up:
  #!/usr/bin/env fish
  for line in (cat ../../.env | grep -v '^#' | grep -v '^[[:space:]]*$')
    set item (string split -m 1 '=' $line)
    set -gx $item[1] $item[2]
  end
  spin up --build --runtime-config-file ../../runtime-config.dev.toml
