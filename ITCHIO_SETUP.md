# itch.io setup and automatic GitHub Releases publishing

Akari-Sweeps uses GitHub Releases as the versioned source of truth. The
Windows Release workflow builds and validates one setup bundle. When the
optional Butler settings are configured, a second GitHub Actions job pushes
that same bundle to one persistent itch.io Windows channel.

This means the itch.io page and its download slot stay in place. Publishing
a new stable GitHub Release updates the existing itch.io channel; there is
no per-release page editing or manual file replacement.

## 1. Create the itch.io project page

1. Sign in to itch.io and open the Creator Dashboard.
2. Choose **Create new game**.
3. Set the title to **Akari-Sweeps**.
4. Choose a short project URL slug such as **akari-sweeps**. The final URL
   will look like https://YOUR_ITCH_USERNAME.itch.io/akari-sweeps.
5. Set the project type to **Downloadable**, not HTML/HTML5.
6. Choose the closest accurate classification. If no category fits this
   Windows utility, choose **Other** and use a custom noun such as
   **Windows utility**.
7. Select Windows as the supported platform. Do not select macOS or Linux;
   this release only supports Windows 10/11 x86_64.
8. Set your intended price model. For a free release with optional support,
   choose the $0 / Donate option; otherwise choose the price you intend.
9. Add a cover image. itch.io recommends a 315:250 aspect ratio (630x500 is
   a good larger size) and recommends 3–5 screenshots.
10. In the description editor, use the copy in ITCHIO_PAGE_COPY.md. Keep
    the permanent GitHub download link and the one-command PowerShell
    installation instructions in the description.
11. Save the page as a draft and preview it before making it public.

The stable fallback download link is:

https://github.com/vishwassharma38/Akari-Sweeps/releases/latest/download/akari-sweeps-itchio-setup.zip

The one-command installer uses this stable latest-release asset:

https://github.com/vishwassharma38/Akari-Sweeps/releases/latest/download/install-release.ps1

Do not replace these links for individual versions. GitHub resolves the
/releases/latest/download/<asset-name> format to an asset with that name
on the latest stable release.

## 2. Configure the one-time GitHub-to-itch.io connection

The workflow uses the official itch.io Butler CLI through the
remarkablegames/setup-butler GitHub Action.

### Get an itch.io API key

1. Open https://itch.io/user/settings/api-keys.
2. Generate an API key for Butler (the key source should be wharf).
3. Copy it once. Treat it like a password. Do not add it to source code,
   a commit, an issue, a PR, or the itch.io page.

Account API keys are long-lived and have broad account access. Store the key
only as a GitHub Actions secret, and revoke it from itch.io settings if it
is ever exposed.

### Add the GitHub secret

1. Open the Akari-Sweeps GitHub repository.
2. Go to **Settings → Secrets and variables → Actions → Secrets**.
3. Choose **New repository secret**.
4. Name it exactly BUTLER_API_KEY.
5. Paste the itch.io API key as its value and save.

### Add the GitHub repository variable

1. Stay under **Settings → Secrets and variables → Actions → Variables**.
2. Choose **New repository variable**.
3. Name it exactly ITCHIO_TARGET.
4. Set its value to:

   YOUR_ITCH_USERNAME/akari-sweeps:windows

Replace YOUR_ITCH_USERNAME with the account part of the itch.io page URL.
Use the project slug after the slash. Keep the channel suffix :windows.

Example: if the page URL is https://vish.itch.io/akari-sweeps, the value is:

vish/akari-sweeps:windows

Save the variable. The secret and variable are configured once.

## 3. Publish the first release

The current Cargo.toml version is 0.1.0. After confirming the page and
the two GitHub Actions settings above:

1. Confirm the main branch contains the release workflow and the installer.
2. From a local checkout with main current, create and push the matching
   tag:

   ~~~powershell
   git checkout main
   git pull origin main
   git tag v0.1.0
   git push origin v0.1.0
   ~~~

3. GitHub Actions runs the Windows release workflow. It validates the
   version/tag match, runs tests, builds the executable, packages the
   application and assets, validates the setup bundle, and publishes the
   GitHub Release assets.
4. After the Release job succeeds, the optional Butler job downloads that
   exact setup bundle and pushes it to the configured windows channel.
5. Check the Actions run and the itch.io project page. The Windows download
   should appear in the itch.io Download area.

If ITCHIO_TARGET is unset, the Butler job is skipped and the stable GitHub
download link remains available. If ITCHIO_TARGET is set but the
BUTLER_API_KEY secret is missing or invalid, the Butler job fails with a
clear error; add/fix the secret and rerun the failed workflow job.

## 4. What happens for future releases

For each later version:

1. Bump the version in Cargo.toml and refresh/commit Cargo.lock.
2. Run the repository's release checks.
3. Push the new matching version tag (for example v0.1.1).

The workflow then:

- Publishes the versioned Windows ZIP, SHA-256 sidecar, installer, and
  self-contained setup bundle to GitHub Releases.
- Pushes the setup bundle to the same itch.io windows channel through
  Butler.
- Updates the existing itch.io download slot. It does not create a new
  project page, change the project URL, or require a manual upload.

The itch.io page's stable GitHub links also continue to resolve to the
latest stable release. They are useful as a fallback and never need to be
edited per version.

## 5. Important distinction about updates

A newer download becoming available on itch.io is not the same thing as
silently updating an already-installed copy of Akari-Sweeps.

The installed application has its own GitHub update check:

- akari update --check checks without installing.
- akari update downloads, verifies, and installs the latest stable release.
- Automatic checks in the TUI only notify; they do not silently replace
  application files.

The itch.io app can use Butler's patching flow for builds published to the
same channel. Users who download files directly from a browser still need
to download the newer version or use akari update.

## Official references

- https://itch.io/docs/creators/getting-started
- https://itch.io/docs/butler/pushing.html
- https://itchio.github.io/butler/login.html
- https://github.com/remarkablegames/setup-butler
- https://docs.github.com/en/repositories/releasing-projects-on-github/linking-to-releases
