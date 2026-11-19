
## Audit fixes
- Permission nodes renamed to the documented `voicechat.listen|speak|groups|admin` (the `horizonvc.*` names are not kept as aliases: the three default-true nodes would make an alias OR-check grant what a `voicechat.*` denial refuses).
- `voicechat test` usages carry the `voicechat.admin` node.
- `join` accepts group names with spaces and case-insensitive names.
- Gaps: spectator interaction/possession, categories, locational/entity audio, hidden groups, translations, `allow_pings` unchanged.
