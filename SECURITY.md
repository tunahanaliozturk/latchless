# Security policy

## Reporting a vulnerability

Report it privately through GitHub: the **Security** tab of this repository, then **Report a vulnerability**. Please do not open a public issue, discussion or pull request for it.

Include what you found and how to reproduce it. You will get an answer within 7 days, and a fix or a plan within 30 days for confirmed issues. You are credited in the advisory unless you ask not to be.

## Supported versions

Security fixes go into the latest release only.

## What counts

Any way to cause undefined behaviour using only safe code and the public API: a data race, a use after free, a double free, a read of uninitialised memory, or a `Send` or `Sync` impl that lets a type cross threads when it must not.

Out of scope: memory growth while a thread stays pinned in `epoch` (documented), and anything that needs the caller to break the documented contract of an `unsafe fn`.
