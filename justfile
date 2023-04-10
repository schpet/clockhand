run *ARGS:
    cargo run -- {{ARGS}}

watch PATH="~/code/*/clockhand.json ~/code/*/.config/clockhand.json":
    cargo run -- watch {{PATH}} -i 5

symlink NAME="~/bin/clockhand":
    ln -s "$(pwd)/target/debug/clockhand" {{NAME}}
