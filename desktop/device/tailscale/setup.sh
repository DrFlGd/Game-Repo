#!/usr/bin/env bash
# Tailscale on the Steam Deck, set up by Game Repo Go (docs/PLAN.md, D179). Part of Game
# Repo, under its MIT licence. Game Repo Go writes this file out and runs it in a terminal
# window as the Deck's own user:
#
#   bash setup.sh               a password for the Deck if it has none, the install (asking
#                               for that password), then Tailscale signed in to
#   sudo bash setup.sh install  the install alone
#
# The install takes the newest stable Tailscale from Tailscale's own server, checked against
# the SHA-256 Tailscale publishes beside it, and puts it where SteamOS's updates leave it:
# its programs in /opt/tailscale, Tailscale's own service in /etc/systemd/system, and those
# files listed in /etc/atomic-update.conf.d, which SteamOS keeps through an update. Run
# again, it puts the newest Tailscale over the one there, keeping its settings and sign-in.
#
# Tests install into a folder instead (GAMEREPO_ROOT) from a stand-in server
# (GAMEREPO_TAILSCALE_PACKAGES): desktop/device/src/tailscale.rs.
set -euo pipefail

root="${GAMEREPO_ROOT:-}"
packages="${GAMEREPO_TAILSCALE_PACKAGES:-https://pkgs.tailscale.com/stable}"
opt=/opt/tailscale
tailscale="$root$opt/tailscale"

step() { printf '\n\033[1m%s\033[0m\n' "$*"; }
stop() { printf '\n%s\n' "$*" >&2; exit 1; }

install_tailscale() {
  [ -n "$root" ] || [ "$(id -u)" = 0 ] || stop "Installing Tailscale needs administrator rights: run this with sudo."
  case "$(uname -m)" in
    x86_64) arch=amd64 ;;
    aarch64 | arm64) arch=arm64 ;;
    *) stop "Tailscale isn't made for this processor ($(uname -m))." ;;
  esac
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT

  step "Finding the newest Tailscale…"
  name="$(curl -fsSL "$packages/?mode=json" | grep -o "\"tailscale_[0-9.]*_${arch}\.tgz\"" | head -n 1 | tr -d '"')" || true
  [ -n "$name" ] || stop "Tailscale's server didn't say which Tailscale is the newest. Check this Deck is online, then try again."
  version="${name#tailscale_}"
  version="${version%_"$arch".tgz}"
  echo "Tailscale $version."

  step "Downloading it from Tailscale…"
  curl -fL --progress-bar -o "$work/$name" "$packages/$name" || stop "Tailscale couldn't be downloaded. Check this Deck is online, then try again."
  published="$(curl -fsSL "$packages/$name.sha256" | awk '{ print $1; exit }')" || true
  got="$(sha256sum "$work/$name" | awk '{ print $1 }')"
  [ -n "$published" ] && [ "$published" = "$got" ] || stop "The download isn't the file Tailscale published (its SHA-256 doesn't match), so nothing was installed."
  tar -xzf "$work/$name" -C "$work"
  from="$work/${name%.tgz}"
  [ -f "$from/tailscale" ] && [ -f "$from/tailscaled" ] && [ -f "$from/systemd/tailscaled.service" ] \
    || stop "Tailscale's download isn't laid out as it used to be, so nothing was installed. Game Repo Go needs an update for it."

  step "Installing it in $opt…"
  # an older way of installing it on the Deck, as a system extension, would hide this one
  if [ -d "$root/var/lib/extensions/tailscale" ]; then
    systemctl stop tailscaled >/dev/null 2>&1 || true
    systemd-sysext unmerge >/dev/null 2>&1 || true
    rm -rf "$root/var/lib/extensions/tailscale"
    systemd-sysext merge >/dev/null 2>&1 || true
  fi
  mkdir -p "$root$opt"
  # each program whole before it takes the old one's place, so a running Tailscale goes on
  for program in tailscale tailscaled; do
    install -m 755 "$from/$program" "$root$opt/.$program.new"
    mv -f "$root$opt/.$program.new" "$root$opt/$program"
  done
  # Tailscale's own service, pointed at its programs here
  mkdir -p "$root/etc/systemd/system" "$root/etc/default" "$root/etc/atomic-update.conf.d"
  sed -e "s#/usr/sbin/tailscaled#$opt/tailscaled#g" -e "s#/usr/bin/tailscaled#$opt/tailscaled#g" \
    "$from/systemd/tailscaled.service" > "$root/etc/systemd/system/tailscaled.service"
  # its settings (the port, any flags) stay as they are once they're there
  if [ ! -s "$root/etc/default/tailscaled" ] && [ -f "$from/systemd/tailscaled.defaults" ]; then
    install -m 644 "$from/systemd/tailscaled.defaults" "$root/etc/default/tailscaled"
  fi
  # what SteamOS keeps through its updates
  cat > "$root/etc/atomic-update.conf.d/gamerepo-tailscale.conf" <<'KEEP'
/etc/systemd/system/tailscaled.service
/etc/systemd/system/multi-user.target.wants/tailscaled.service
/etc/default/tailscaled
/etc/atomic-update.conf.d/gamerepo-tailscale.conf
KEEP
  # `tailscale` in a terminal, where SteamOS lets it be put
  { mkdir -p "$root/usr/local/bin" && ln -sf "$opt/tailscale" "$root/usr/local/bin/tailscale"; } 2>/dev/null || true

  step "Starting it…"
  systemctl daemon-reload
  systemctl enable tailscaled >/dev/null 2>&1 || stop "Tailscale's service couldn't be turned on."
  systemctl restart tailscaled || stop "Tailscale's service didn't start."
  echo "Tailscale $version is installed."
}

# What Tailscale's doing: Running, NeedsLogin, Stopped… (empty while it doesn't answer)
backend_state() {
  "$tailscale" status --json 2>/dev/null | grep -o '"BackendState": *"[A-Za-z]*"' | head -n 1 | sed 's/.*"\([A-Za-z]*\)"$/\1/' || true
}

# The window stays open to be read, whatever happened.
finish() {
  [ -n "$done" ] || printf "\nTailscale isn't set up: see above. Game Repo Go can run this again.\n"
  if [ -t 0 ]; then read -r -p "Press Enter to close this window. " _ || true; fi
}

set_up() {
  [ -n "$root" ] || [ "$(id -u)" != 0 ] || stop "Run this as the Deck's own user, not as root: it asks for the password itself."
  me="$(id -un)"
  done=""
  trap finish EXIT

  step "Setting up Tailscale on this Steam Deck, for Game Repo Go"
  echo "Tailscale lets Game Repo Go reach your library when you're away from home."

  # SteamOS starts with no password, and installing needs one
  if [ "$(passwd -S "$me" 2>/dev/null | awk '{ print $2 }')" = "NP" ]; then
    step "First, a password for this Deck"
    echo "Installing Tailscale needs administrator rights, which need a password, and this Deck has none yet."
    echo "Choose one and type it twice below (nothing shows as you type). Keep it: SteamOS asks for it now and then."
    passwd
  fi

  step "Your Deck's password, to install Tailscale"
  echo "(If you've never set one, press Ctrl+C, run passwd in Konsole to choose one, then set up Tailscale again.)"
  sudo bash "$0" install

  step "Signing in to Tailscale"
  # the Deck's own user may then turn Tailscale on and off without the password
  sudo "$tailscale" set --operator="$me"
  if [ "$(backend_state)" = "Running" ]; then
    echo "Tailscale is already signed in."
  else
    echo "Game Repo Go opens Tailscale's sign-in page in your browser (the link is below too)."
    echo "Sign in with the Tailscale account your library's server uses; this carries on once you have."
    "$tailscale" up
  fi
  done=1
  step "Tailscale is on"
  ip="$("$tailscale" ip -4 2>/dev/null | head -n 1)" || true
  echo "This Deck is ${ip:-on your Tailscale network}. Game Repo Go reaches your library through it when you're away from home."
}

case "${1:-}" in
  install) install_tailscale ;;
  "") set_up ;;
  *) stop "Run it as: bash setup.sh   (or: sudo bash setup.sh install)" ;;
esac
