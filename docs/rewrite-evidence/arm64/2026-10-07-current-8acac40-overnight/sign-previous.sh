#!/bin/sh
# Sign a disposable test repository around one unchanged earlier package.
set -eu
deb="$1"
apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends gnupg apt-utils > /dev/null
gpg --batch --pinentry-mode loopback --passphrase '' \
  --quick-generate-key 'Disposable previous-artifact key <rewrite@test.invalid>' rsa2048 sign 1d
fpr=$(gpg --batch --with-colons --list-secret-keys | awk -F: '/^fpr:/{print $10; exit}')
bash /root/qual/source/packaging/repository.sh /opt/limeos-previous-repo "$fpr" "$deb"
echo "$fpr" > /root/qual/previous-signing-fingerprint.txt
sha256sum "$deb" /opt/limeos-previous-repo/pool/main/*.deb
