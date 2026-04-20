#!/usr/bin/env nu

let page_access_token = (input "Meta Page Access Token: ")
let page_id = (input "Meta Page ID: ")
let ig_user_id = (input "Instagram User ID: ")
let threads_user_id = (input "Threads User ID: ")
let threads_access_token = (input "Threads Access Token (from Threads OAuth, NOT the Page token): ")

print ""
print "Setting GitHub secrets for gluebox CI/CD..."

$page_access_token | gh secret set META_PAGE_ACCESS_TOKEN --repo koalazub/gluebox
$page_id | gh secret set META_PAGE_ID --repo koalazub/gluebox
$ig_user_id | gh secret set META_IG_USER_ID --repo koalazub/gluebox
$threads_user_id | gh secret set META_THREADS_USER_ID --repo koalazub/gluebox
$threads_access_token | gh secret set META_THREADS_ACCESS_TOKEN --repo koalazub/gluebox

print "GitHub secrets set."
print ""
print "Writing to gluebox prod config..."

let toml_lines = [
    ""
    "[stonkwatch_social.meta]"
    $'page_access_token = "($page_access_token)"'
    $'page_id = "($page_id)"'
    $'ig_user_id = "($ig_user_id)"'
    $'threads_user_id = "($threads_user_id)"'
    $'threads_access_token = "($threads_access_token)"'
]

let toml_block = ($toml_lines | str join "\n")

$toml_block | ssh gluebox "sudo tee -a /etc/gluebox/gluebox.toml > /dev/null"

print "Prod config updated."
print ""
print "Restarting gluebox..."

ssh gluebox "sudo systemctl restart gluebox"

print "Done. Checking logs..."
sleep 5sec

ssh gluebox "journalctl -u gluebox -n 20 --no-pager"
