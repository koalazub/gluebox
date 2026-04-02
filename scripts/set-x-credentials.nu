#!/usr/bin/env nu

let client_id = (input "X OAuth 2.0 Client ID: ")
let client_secret = (input "X OAuth 2.0 Client Secret: ")

let code_verifier = (random chars --length 64)
let state = (random chars --length 16)

let auth_url = $"https://twitter.com/i/oauth2/authorize?response_type=code&client_id=($client_id)&redirect_uri=https%3A%2F%2Fstonkwatch.app%2Fauth%2Fcallback&scope=tweet.read%20tweet.write%20users.read%20offline.access&state=($state)&code_challenge=($code_verifier)&code_challenge_method=plain"

print ""
print "Open this URL in your browser and authorize the app:"
print ""
print $auth_url
print ""
print "After authorizing, you'll be redirected. Copy the 'code' parameter from the URL."
print ""

let auth_code = (input "Paste the authorization code: ")

print ""
print "Exchanging code for tokens..."

let token_response = (http post "https://api.twitter.com/2/oauth2/token"
    --content-type "application/x-www-form-urlencoded"
    --user $"($client_id):($client_secret)"
    --headers [Accept "application/json"]
    $"grant_type=authorization_code&code=($auth_code)&redirect_uri=https%3A%2F%2Fstonkwatch.app%2Fauth%2Fcallback&code_verifier=($code_verifier)"
)

let access_token = ($token_response | get access_token)
let refresh_token = ($token_response | get refresh_token)

print $"Access token: ($access_token | str substring 0..20)..."
print $"Refresh token: ($refresh_token | str substring 0..20)..."
print ""
print "Setting GitHub secrets..."

$client_id | gh secret set X_CLIENT_ID --repo koalazub/gluebox
$client_secret | gh secret set X_CLIENT_SECRET --repo koalazub/gluebox
$access_token | gh secret set X_ACCESS_TOKEN --repo koalazub/gluebox
$refresh_token | gh secret set X_REFRESH_TOKEN --repo koalazub/gluebox

print "GitHub secrets set."
print ""
print "Updating gluebox prod config..."

let toml_block = [
    "[stonkwatch_social.x]"
    $'client_id = "($client_id)"'
    $'client_secret = "($client_secret)"'
    $'access_token = "($access_token)"'
    $'refresh_token = "($refresh_token)"'
] | str join "\n"

ssh gluebox "sudo sed -i '/\\[stonkwatch_social\\.x\\]/,/^\\[/{/^\\[stonkwatch_social\\.x\\]/d;/^\\[/!d}' /etc/gluebox/gluebox.toml"
$toml_block | ssh gluebox "sudo tee -a /etc/gluebox/gluebox.toml > /dev/null"

print "Prod config updated."
print ""
print "Restarting gluebox..."

ssh gluebox "sudo systemctl restart gluebox"

print "Done. Checking logs..."
sleep 5sec

ssh gluebox "journalctl -u gluebox -n 20 --no-pager"
