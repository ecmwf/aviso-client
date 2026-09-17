<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Python

Use `pyaviso` to receive Aviso notifications in your Python scripts. For
example, your script can wait until a dataset is ready, then start processing
it.

> **For users:** listen for notifications or replay past ones. You do not need
> to publish anything yourself.
>
> **For notification providers:** publish notifications to tell users that data
> or an event is available. Publishing requires permission on the server.

## What you can do

- **See what is available:** discover the server's notification types and their
  filter fields.
- **Listen for updates:** receive notifications matching the data you care
  about.
- **Read past notifications:** replay available history.
- **React to a notification:** run your Python code, write a log, or call
  another service.

Providers can also publish notifications from Python. A notification can
include a file location or other useful information; publishing it does not
transfer the file.

## Start here

1. [Install pyaviso](./install.md).
2. Follow the [Python quickstart](./quickstart.md) to connect to a server,
   inspect its schema and start listening.

You will need your Aviso server's address and, if required, credentials. The
server's schemas determine which notification types and filters you can use.

## When you need more

- [Listening](./listen.md): filters and reading notifications.
- [Publishing](./publish.md): sending notifications as a provider.
- [State and resume](./state-and-resume.md): remembering progress between runs.
- [Triggers](./triggers.md): running actions for matching notifications.
- [Authentication](./auth.md) and [Troubleshooting](./troubleshooting.md):
  connection and access help.
- [API reference](./api-reference.md): details of the available methods.

If your application already uses Python's `asyncio`, see [Async](./async.md).
Otherwise, start with the regular client used in the quickstart.
