# Design notes

This book shows how to use Elide. The design behind it, and the reasons for
it, are written up in the repository's
[`docs/`](https://github.com/nvisycom/elide/tree/main/docs):

| Note | Subject |
|---|---|
| [Ingestion](https://github.com/nvisycom/elide/blob/main/docs/INGESTION.md) | How bytes become addressable parts: format resolution, streaming, and lifting chunk coordinates to source coordinates |
| [Detection](https://github.com/nvisycom/elide/blob/main/docs/DETECTION.md) | Why several recognition techniques, and how their findings are reconciled |
| [Redaction](https://github.com/nvisycom/elide/blob/main/docs/REDACTION.md) | The operator taxonomy, leak profiles, and the boundary of reversibility |
| [Comparison](https://github.com/nvisycom/elide/blob/main/docs/COMPARISON.md) | How Elide relates to Microsoft Presidio, and where each is ahead |
