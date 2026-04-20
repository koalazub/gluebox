#!/usr/bin/env nu

# Sets the Meta (Facebook / Instagram / Threads) credentials as GitHub Actions
# secrets. The deploy workflow renders `/etc/gluebox/gluebox.toml` from these
# secrets on every push to main, so after running this script either push a
# commit to main or trigger the workflow manually with `gh workflow run
# "Deploy gluebox-prod"` to sync the VPS.

let page_access_token = (input "Meta Page Access Token: ")
let page_id = (input "Meta Page ID: ")
let ig_user_id = (input "Instagram User ID: ")
let threads_user_id = (input "Threads User ID: ")
let threads_access_token = (input "Threads Access Token (from Threads OAuth, NOT the Page token): ")

print ""
print "Setting GitHub secrets..."

$page_access_token | gh secret set META_PAGE_ACCESS_TOKEN --repo koalazub/gluebox
$page_id | gh secret set META_PAGE_ID --repo koalazub/gluebox
$ig_user_id | gh secret set META_IG_USER_ID --repo koalazub/gluebox
$threads_user_id | gh secret set META_THREADS_USER_ID --repo koalazub/gluebox
$threads_access_token | gh secret set META_THREADS_ACCESS_TOKEN --repo koalazub/gluebox

print ""
print "Secrets set. Trigger a deploy to sync the VPS:"
print "  gh workflow run 'Deploy gluebox-prod' --repo koalazub/gluebox"
