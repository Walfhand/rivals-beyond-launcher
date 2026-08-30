# Privacy policy

The Rivals Beyond launcher does not include telemetry, advertising, analytics or user tracking.

It makes HTTPS requests to the Rivals Beyond object-storage service to:

- check signed launcher, news and game-client manifests;
- download files when the user installs, updates or repairs the client;
- download a launcher update when a newer signed version is available.

These requests necessarily expose the user's IP address and standard HTTP metadata to the storage provider. The launcher does not upload account credentials, game settings, screenshots, logs, file contents or the selected installation path.

The selected client directory and the last valid signed news response are stored locally. The launcher starts the locally installed game executable only after the user chooses **Play**.
