#!/usr/bin/env python3
"""One-shot TikTok Desktop OAuth callback with PKCE and Keychain storage."""
import hashlib, http.server, json, os, secrets, subprocess, urllib.error, urllib.parse, urllib.request

CLIENT_KEY=os.environ["SOCIALFLOW_TIKTOK_CLIENT_KEY"]
CLIENT_SECRET=os.environ["SOCIALFLOW_TIKTOK_CLIENT_SECRET"]
STATE=secrets.token_urlsafe(32)
VERIFIER=secrets.token_urlsafe(64)[:96]
CHALLENGE=hashlib.sha256(VERIFIER.encode()).hexdigest()
result={}

class Callback(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        query=urllib.parse.parse_qs(urllib.parse.urlparse(self.path).query)
        if query.get("state",[""])[0] != STATE:
            result["error"]="TikTok returned an invalid security state"
        elif query.get("error"):
            result["error"]=query.get("error_description",query["error"])[0]
        else:
            result["code"]=query.get("code",[""])[0]
        body=b"<h2>SocialFlow received the TikTok response.</h2><p>You can close this window and return to SocialFlow.</p>"
        self.send_response(200); self.send_header("Content-Type","text/html"); self.send_header("Content-Length",str(len(body))); self.end_headers(); self.wfile.write(body)
    def log_message(self,*_): pass

server=http.server.HTTPServer(("127.0.0.1",0),Callback)
redirect=f"http://127.0.0.1:{server.server_port}/callback/"
params={"client_key":CLIENT_KEY,"response_type":"code","scope":"user.info.basic,video.list,video.upload,video.publish","redirect_uri":redirect,"state":STATE,"code_challenge":CHALLENGE,"code_challenge_method":"S256"}
subprocess.run(["/usr/bin/open","https://www.tiktok.com/v2/auth/authorize/?"+urllib.parse.urlencode(params)],check=True)
server.timeout=300
server.handle_request(); server.server_close()
if result.get("error") or not result.get("code"):
    raise SystemExit(result.get("error","TikTok did not return an authorization code"))
data=urllib.parse.urlencode({"client_key":CLIENT_KEY,"client_secret":CLIENT_SECRET,"code":urllib.parse.unquote(result["code"]),"grant_type":"authorization_code","redirect_uri":redirect,"code_verifier":VERIFIER}).encode()
req=urllib.request.Request("https://open.tiktokapis.com/v2/oauth/token/",data=data,method="POST"); req.add_header("Content-Type","application/x-www-form-urlencoded")
with urllib.request.urlopen(req,timeout=60) as response: tokens=json.loads(response.read())
if tokens.get("error"): raise SystemExit(tokens.get("error_description",tokens["error"]))
access=tokens["access_token"]; refresh=tokens["refresh_token"]; open_id=tokens["open_id"]
for service,account,password in [("com.socialflow.desktop.tiktok",open_id,access),("com.socialflow.desktop.tiktok.refresh",open_id,refresh),("com.socialflow.desktop.tiktok.client",CLIENT_KEY,CLIENT_SECRET)]:
    subprocess.run(["/usr/bin/security","add-generic-password","-U","-s",service,"-a",account,"-w",password],check=True,stdout=subprocess.DEVNULL)
profile={}
try:
    info=urllib.request.Request("https://open.tiktokapis.com/v2/user/info/?fields=open_id,display_name,avatar_url")
    info.add_header("Authorization",f"Bearer {access}")
    with urllib.request.urlopen(info,timeout=45) as response:
        profile=json.loads(response.read()).get("data",{}).get("user",{})
except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError, json.JSONDecodeError):
    # Profile decoration is optional; a valid OAuth exchange must still connect.
    pass
print(json.dumps({"open_id":open_id,"display_name":profile.get("display_name") or "TikTok account","scope":tokens.get("scope",""),"expires_in":tokens.get("expires_in",86400),"refresh_expires_in":tokens.get("refresh_expires_in",31536000)}))
