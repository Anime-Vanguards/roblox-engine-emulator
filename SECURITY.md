# Security policy

## Reporting a vulnerability

Please report security problems privately through GitHub: on this repository's **Security** tab, choose **Report a vulnerability** ([direct link](https://github.com/Anime-Vanguards/roblox-engine-emulator/security/advisories/new)). Only the maintainers can see the report.

Please don't open a public issue, PR or discussion about it.

Include what you found, how to reproduce it, which commit you tested, and what an attacker could do with it. A maintainer will reply within a week and keep you updated until it's fixed. We're glad to credit you in the advisory unless you'd rather stay anonymous.

## Supported versions

Only the latest commit on `main` gets security fixes. There are no release branches yet.

## Scope

In scope: the code in this repository, its build scripts and its CI workflows. Examples:

- a crafted model, place, script or mesh file that makes the tool run code, write outside its output folder, or read files it shouldn't;
- a way for the local preview or render service to be reached from other machines or websites;
- anything that could leak a credential the tool is given.

Out of scope: the Roblox platform, Roblox accounts, any Roblox game or experience, and issues in third-party dependencies that are already public (report those upstream).

## Credentials

ree reads no Roblox cookie, token or key, and its scripts cannot reach the network (script HTTP requests are disabled). The optional asset downloads (manifest bundles and animations) go through an external asset worker that `REE_ASSET_WORKER` names: the worker handles any authentication itself, and ree only sends it JSON requests on stdin, so ree never sees a credential. `ree serve` listens on 127.0.0.1 only and has no authentication, so run it only on a machine you don't share with other users.

Never paste a cookie, token or key into an issue, PR or log. If you posted one by accident, treat it as stolen: for a Roblox cookie, sign out of all sessions in your Roblox account settings, which invalidates it, then tell us so we can delete the post.
