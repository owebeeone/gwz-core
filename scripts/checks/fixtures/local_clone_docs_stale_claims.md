# Deliberately stale local clone claims

Create a lane with `gwz clone --local --name A ../gwz-dev-A`. `--clean` and
`--bare` are now supported, and family pull and push are served by this
build. Ordinary dispose archives the lane before deleting it, so nothing is
lost.
