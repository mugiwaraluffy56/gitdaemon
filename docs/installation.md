# Installation

---

## Requirements

| Requirement | Minimum version | Notes |
|---|---|---|
| Rust | 1.75 | `rustup update stable` |
| OS | macOS 12+ / Linux (glibc 2.31+) | Windows not supported (Unix sockets) |
| Git | 2.38+ | Must be configured with `user.name` and `user.email` |
| SSH key | — | Required for SSH remotes (`~/.ssh/id_ed25519` or `id_rsa`) |

`libgit2` is bundled by the `git2` crate — no separate system package needed.

---

## From pre-built binary (recommended)

Download the latest release from the
[GitHub releases page](https://github.com/mugiwaraluffy56/gitdaemon/releases).

```sh
# macOS (Apple Silicon)
curl -L https://github.com/mugiwaraluffy56/gitdaemon/releases/latest/download/gd-aarch64-apple-darwin.tar.gz \
  | tar xz
sudo mv gd /usr/local/bin/

# macOS (Intel)
curl -L https://github.com/mugiwaraluffy56/gitdaemon/releases/latest/download/gd-x86_64-apple-darwin.tar.gz \
  | tar xz
sudo mv gd /usr/local/bin/

# Linux (x86_64)
curl -L https://github.com/mugiwaraluffy56/gitdaemon/releases/latest/download/gd-x86_64-unknown-linux-gnu.tar.gz \
  | tar xz
sudo mv gd /usr/local/bin/

# Verify
gd --version
```

---

## From source

```sh
git clone https://github.com/mugiwaraluffy56/gitdaemon
cd gitdaemon
cargo build --release
cp target/release/gd ~/.local/bin/
```

Or install directly with cargo:

```sh
cargo install --git https://github.com/mugiwaraluffy56/gitdaemon --locked
```

---

## Shell completions

### Bash

```sh
gd completions bash >> ~/.bashrc
source ~/.bashrc
```

### Zsh

```sh
gd completions zsh > ~/.zfunc/_gd
echo 'fpath=(~/.zfunc $fpath)' >> ~/.zshrc
echo 'autoload -U compinit && compinit' >> ~/.zshrc
source ~/.zshrc
```

### Fish

```sh
gd completions fish > ~/.config/fish/completions/gd.fish
```

---

## Updating

```sh
# From source
cd gitdaemon && git pull && cargo build --release && cp target/release/gd ~/.local/bin/

# Via cargo install
cargo install --git https://github.com/mugiwaraluffy56/gitdaemon --locked --force
```

---

## Uninstalling

```sh
gd down          # stop any running daemon first
rm $(which gd)
```

---

## Verifying the install

```sh
gd --version     # prints version
gd --help        # prints command reference
cd /any/git/repo
gd init          # creates gd.yml
gd up            # starts daemon (Ctrl-C to stop)
```
