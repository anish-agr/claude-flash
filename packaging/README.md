# Packaging

| Folder | Package manager | Where it is published |
|---|---|---|
| `homebrew` | Homebrew, macOS | Copied to `Formula/claude-flash.rb` in [anish-agr/homebrew-tap](https://github.com/anish-agr/homebrew-tap) |
| `scoop` | Scoop, Windows | Read straight from this repository, by URL |
| `winget` | WinGet, Windows | A pull request to [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) |

`scripts/update-packages.sh VERSION` points all of them at a published release.
Homebrew and Scoop need nothing else described here; WinGet does.

## WinGet

`winget` holds the package as the Windows Package Manager community repository
expects it: a version manifest, an installer manifest and an English locale
manifest, and nothing else, because the tools read every file in that folder.

WinGet installs the release zip as a *portable* package: it unpacks the archive into
`%LOCALAPPDATA%\Microsoft\WinGet\Packages\anish-agr.ClaudeFlash_…` and puts a `flash`
alias on `PATH`. `flash-agent.exe` is unpacked alongside it, which is where `flash`
looks for it. `flash install` recognises that folder and leaves the programs where
WinGet put them, so `winget upgrade` replaces both of them in place.

## Publishing a version

WinGet packages live in [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs);
publishing means opening a pull request there. Do this after the GitHub release
exists, because the manifest carries the release's URL and checksum.

1. Make sure the manifests point at the version you are publishing:

   ```bash
   scripts/update-packages.sh 2.3.0
   ```

2. Check them against the schema. This needs nothing but the `winget` that ships
   with Windows:

   ```powershell
   winget validate --manifest packaging\winget
   ```

3. Install the submission tool once:

   ```powershell
   winget install Microsoft.WingetCreate
   ```

4. Submit. `wingetcreate` forks `winget-pkgs` for you, puts the files in
   `manifests/a/anish-agr/ClaudeFlash/2.3.0/` and opens the pull request:

   ```powershell
   wingetcreate submit --token <github-token> packaging\winget
   ```

   The token needs the `public_repo` scope. Without one, `wingetcreate` asks GitHub
   for a device code and you sign in through the browser.

   An automated pipeline then checks the manifest, downloads the archive and
   installs it in a sandbox, and comments on the pull request with what it found. A
   package nobody has published before is also looked at by a person, so the first
   submission takes longer than the ones after it.

5. Once the pull request is merged and `winget show anish-agr.ClaudeFlash` finds the
   package, add it to the README's install section, next to Homebrew and Scoop:

   ````markdown
   On Windows, with WinGet, which ships with Windows 11:

   ```powershell
   winget install anish-agr.ClaudeFlash
   ```
   ````

   Until then the command does not work, so it does not belong in the README.

## Updating an existing version

Once `anish-agr.ClaudeFlash` is in the repository, later versions need neither this
folder nor a fork by hand:

```powershell
wingetcreate update anish-agr.ClaudeFlash --version 2.4.0 --urls <release-zip-url> --submit --token <github-token>
```

Keep the files here in step anyway, so the repository always shows what was
published.

## What WinGet does not fix

Being in WinGet does not make the programs trusted: Smart App Control judges the
executables themselves, so a user who has it on is blocked whether they install with
WinGet, with Scoop or by hand. [SECURITY.md](../SECURITY.md#code-signing) covers
the signing that does fix it.
