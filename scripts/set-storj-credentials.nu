#!/usr/bin/env nu

let access_key = (input "Storj S3 Access Key: ")
let secret_key = (input "Storj S3 Secret Key: ")
let bucket = (input --default "stonkwatch-og" "Storj Bucket Name (default: stonkwatch-og): ")

print ""
print "Setting GitHub secrets..."

$access_key | gh secret set STORJ_ACCESS_KEY --repo koalazub/gluebox
$secret_key | gh secret set STORJ_SECRET_KEY --repo koalazub/gluebox
$bucket | gh secret set STORJ_BUCKET --repo koalazub/gluebox

print "GitHub secrets set."
print ""
print "Updating gluebox prod config..."

let toml_block = [
    ""
    "[stonkwatch_social.storj]"
    $'access_key = "($access_key)"'
    $'secret_key = "($secret_key)"'
    $'bucket = "($bucket)"'
] | str join "\n"

$toml_block | ssh gluebox "sudo tee -a /etc/gluebox/gluebox.toml > /dev/null"

print "Prod config updated."
print ""
print "Restarting gluebox..."

ssh gluebox "sudo systemctl restart gluebox"

print "Done. Checking logs..."
sleep 5sec

ssh gluebox "journalctl -u gluebox -n 20 --no-pager"
