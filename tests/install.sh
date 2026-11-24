#!/usr/bin/env bash
# Vérifie sur Linux et macOS qu'un binaire téléchargé s'installe seul : une copie nommée lcs dans
# ~/.local/bin, et ce dossier ajouté au PATH par le profil du shell. Dossier personnel d'essai,
# rien n'est écrit dans le vrai.
# Usage : tests/install.sh <binaire>
set -euo pipefail

bin=$1
work=$(mktemp -d)
home="$work/home"
mkdir -p "$home" "$work/dl"
download="$work/dl/lcs-download-x64"
cp "$bin" "$download"

case "$(uname -s)" in
  Darwin) shell=/bin/zsh profile=.zshenv ;;
  *) shell=/bin/bash profile=.bashrc ;;
esac
# Un terminal neuf : aucun lcs dans le PATH.
fresh() { env -i HOME="$home" SHELL="$shell" PATH=/usr/bin:/bin "$@"; }
terminal() {
  if [ "$profile" = .zshenv ]; then fresh zsh -c "$1"; else fresh bash -c ". \"\$HOME/.bashrc\" && $1"; fi
}

expected=$("$bin" --version)

echo "== première commande du binaire téléchargé"
fresh "$download" --version
test -x "$home/.local/bin/lcs"
grep -q '.local/bin' "$home/$profile"

echo "== un nouveau terminal trouve lcs"
got=$(terminal 'lcs --version')
[ "$got" = "$expected" ] || { echo "lcs --version : « $got », attendu « $expected »"; exit 1; }

echo "== relancé, il ne change plus rien"
before=$(cat "$home/$profile")
fresh "$download" --version
terminal 'lcs install'
[ "$before" = "$(cat "$home/$profile")" ] || { echo "profil modifié une seconde fois"; exit 1; }

echo "== sans commande, dans un autre dossier personnel"
home="$work/home2"
mkdir -p "$home"
fresh "$download"
test -x "$home/.local/bin/lcs"
grep -q '.local/bin' "$home/$profile"

rm -rf "$work"
echo "Installation vérifiée."
