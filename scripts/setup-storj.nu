#!/usr/bin/env nu

let access_grant = (input "Storj Access Grant: ")

print ""
print "Importing access grant..."

uplink access import gluebox $access_grant

print "Access grant imported."
print ""
print "Creating bucket stonkwatch-og..."

uplink mb sj://stonkwatch-og --access gluebox

print ""
print "Registering S3 credentials..."

let s3_output = (uplink access register --access gluebox --public | lines)

let access_key = ($s3_output | where ($it | str contains "Access Key") | first | split column ":" | get column2 | first | str trim)
let secret_key = ($s3_output | where ($it | str contains "Secret Key") | first | split column ":" | get column2 | first | str trim)

print $"Access Key: ($access_key)"
print $"Secret Key: ($secret_key)"
print ""
print "Setting GitHub secrets..."

$access_key | gh secret set STORJ_ACCESS_KEY --repo koalazub/gluebox
$secret_key | gh secret set STORJ_SECRET_KEY --repo koalazub/gluebox
"stonkwatch-og" | gh secret set STORJ_BUCKET --repo koalazub/gluebox

print "GitHub secrets set."
print ""
print "Updating gluebox prod config..."

let toml_block = [
    ""
    "[stonkwatch_social.storj]"
    $'access_key = "($access_key)"'
    $'secret_key = "($secret_key)"'
    $'bucket = "stonkwatch-og"'
] | str join "\n"

$toml_block | ssh gluebox "sudo tee -a /etc/gluebox/gluebox.toml > /dev/null"

print "Prod config updated."
print ""
print "Restarting gluebox..."

ssh gluebox "sudo systemctl restart gluebox"

print "Done. Checking logs..."
sleep 5sec

ssh gluebox "journalctl -u gluebox -n 20 --no-pager"
