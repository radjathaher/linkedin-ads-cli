# linkedin-ads-cli

Auto-generated-ish LinkedIn Marketing API CLI (Rest.li `/rest`). Designed for LLM discovery + scripting.

## Install

### Install script (macOS arm64 + Linux x86_64)

```bash
curl -fsSL https://raw.githubusercontent.com/radjathaher/linkedin-ads-cli/main/scripts/install.sh | bash
```

### Download from GitHub releases

Grab the latest `linkedin-ads-cli-<version>-<os>-<arch>.tar.gz`, unpack, and move `linkedin-ads` to your PATH.

## Auth

Required:

```bash
export LINKEDIN_ACCESS_TOKEN="..."
```

Optional:

```bash
export LINKEDIN_VERSION="202601"                 # defaults to CLI schema default
export LINKEDIN_AD_ACCOUNT_ID="123456"           # default for ad-account commands
export LINKEDIN_ASSET_ID="C5405AQEOFHXqeM2vRA"   # default for asset get
export LINKEDIN_BASE_URL="https://api.linkedin.com/rest"
export LINKEDIN_RESTLI_PROTOCOL_VERSION="2.0.0"
```

### How to get `LINKEDIN_ACCESS_TOKEN`

1. Create a LinkedIn app in the LinkedIn Developer Portal.
2. Request access to the Marketing/Ads APIs (LinkedIn Marketing Developer Platform).
3. Use OAuth2 Authorization Code flow to mint an access token with the required scopes (commonly `rw_ads`, `r_ads_reporting`, etc; exact scopes depend on your program approvals).
4. Paste the token into `LINKEDIN_ACCESS_TOKEN`.

## Discovery

```bash
linkedin-ads list --json
linkedin-ads describe ad-account get --json
linkedin-ads tree --json
```

## Examples

Get ad account:

```bash
linkedin-ads ad-account get --id 123456 --pretty
```

Search ad accounts:

```bash
linkedin-ads ad-account search \
  --params '{"search":"(status:(values:List(ACTIVE)))","sort":"(field:ID,order:DESCENDING)"}' \
  --pretty
```

Create campaign group (payload omitted; see LinkedIn docs):

```bash
linkedin-ads ad-account create-campaign-group --id 123456 --params '{...}' --pretty
```

Ad analytics (use unencoded URNs; CLI will encode query params):

```bash
linkedin-ads ad-analytics analytics \
  --params '{"pivot":"CREATIVE","timeGranularity":"ALL","dateRange":"(start:(year:2025,month:1,day:1))","campaigns":"List(urn:li:sponsoredCampaign:1234567)"}' \
  --pretty
```

Upload image (Assets API):

```bash
linkedin-ads image upload \
  --owner urn:li:organization:24141830 \
  --file ./creative.png \
  --pretty
```

Upload video (Assets API; multipart auto for >200MB):

```bash
linkedin-ads video upload \
  --owner urn:li:organization:24141830 \
  --file ./video.mp4 \
  --wait \
  --pretty
```

Raw call:

```bash
linkedin-ads raw GET /adAccounts/123456 --pretty
```

## Notes

- Query tunneling: long GET URLs may fail; use `--tunnel always` to force POST+`X-HTTP-Method-Override` tunneling.
- `--raw` includes `status` + `headers` + `body`. Useful for create calls that return `x-restli-id`.
- File inputs accept: `@/path/to/file`, `file:///path/to/file`, `https://...`, `s3://bucket/key`, or plain local path.

