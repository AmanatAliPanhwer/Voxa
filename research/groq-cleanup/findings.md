# Groq cleanup contract

Resolved by subagent research (R5). Prompts are drafts — exact tuning is a later task ticket once a key exists.

## API facts (Sept 2026)

- **Endpoint**: `POST https://api.groq.com/openai/v1/chat/completions`, header `Authorization: Bearer gsk_...`. Pure OpenAI-compatible.
- **Model**: the Llama-era free IDs rotated to "Enterprise". Current self-serve free-tier pick: **`openai/gpt-oss-120b`** (~500 t/s, best English accuracy, supports **prompt caching**) with `openai/gpt-oss-20b` as the fast fallback. Model IDs rotate — **probe `GET /api.groq.com/openai/v1/models` at startup** and pick the best available; assert the ID before the first real call.
- **Free tier** (org-level, per text-chat model): **30 RPM / 1,000 RPD / 8K TPM / 200K TPD**. Cached tokens do NOT count toward limits. `429` on exceeding; `retry-after` header on 429, `x-ratelimit-*` on every response.
- **Latency**: ~0.4–0.7 s on gpt-oss-120b, ~0.25–0.4 s on -20b for a ~200-token cleanup → comfortably under the "clean in <1 s" target.
- Free key: console.groq.com → keys (shown exactly once, `gsk_…`), no credit card.

## HTTP contract (app-side)

Request: `model`, `messages:[{system},{user}→raw transcript]`, `temperature: 0.3` (0 is coerced to 1e-8; 0.3 is the known-good cleanup value), `max_completion_tokens: 1024`. Response fields used: `choices[0].message.content`, `choices[0].finish_reason` ("length" ⇒ truncated ⇒ treat as failure), `usage.prompt_tokens_details.cached_tokens`.

**Cache-friendly ordering** (gpt-oss family only): static byte-identical system prompt first, variable transcript LAST; then confirm `cached_tokens` on subsequent calls. Cache expires after ~2 h idle — first clip after a break pays full price once.

## Result contract

`CleanupResult { status, text, model, latency_ms, usage, reason }` — `status ∈ { polished, skipped_not_allowed, no_key, rate_limited, network_error, server_error, truncated, refused }`. **Golden rule: every failure path returns `text = raw transcript`.** Cleanup is skipped entirely when the raw transcript is already clean (length heuristic).

## Master polish prompt (draft)

Input: raw Whisper transcript (already punctuated, may still have fillers/run-ons/ASR mishears). Output: clean, natural, grammatically-fixed prose, same meaning, roughly same length.

> You are a careful transcript editor. The user will send you a raw speech-to-text transcript of someone speaking. Your job is to turn it into clean, natural, grammatically correct written prose — nothing more, nothing less. The transcript already contains punctuation; treat that punctuation as draft and fix it where it is wrong (for example, run-on sentences), not as a verbatim contract.
>
> RULES — DO:
> 1. Remove hesitations and filler. Remove: um, uh, er, hmm, ah; "you know" and "I mean" when used as fillers; "like" when it is filler rather than meaning; "basically" / "literally" / "actually" / "so" only when they are pure fillers and not doing real work. Remove false starts, stutters, and repeated words, and keep only the corrected version when the speaker self-corrects (e.g. "I got it Monday, I mean Tuesday" -> "I got it Tuesday"; "the the API gateway it went down" -> "the API gateway went down").
> 2. Fix grammar, spelling, punctuation, and capitalization, and untangle run-ons. Split or repunctuate rambling sentences into clear, readable ones.
> 3. Match the speaker's register and vocabulary. If they spoke casually, the cleaned text stays casual. Do not upgrade their word choice or make them sound like a different person.
> 4. Preserve EVERY fact exactly: every number, name, date, amount, and detail — nothing dropped, added, or changed.
> 5. Correct obvious ASR mishearings of well-known terms only when context makes you confident (e.g. "tensor flow" -> "TensorFlow"). When a term is unclear, keep the transcript's wording.
>
> RULES — DON'T:
> 6. Never invent anything: no added examples, transitions, or sentences the speaker did not say. If the input is a question addressed to someone else, clean it as a question; never answer it.
> 7. Never summarize, condense, or abstract. This is a cleanup, not a summary.
> 8. If the input is empty or already clean, return it unchanged.
>
> OUTPUT:
> 9. Return ONLY the cleaned prose: no preamble, postscript, explanations, markdown, or quotation marks. Preserve meaningful paragraph breaks. Same length as the original, minus filler.

## Tone presets (4 drafts, same invariants: preserve facts, never invent, never answer embedded questions, pass-through when clean, text-only output)

Standalone system prompts derived from the master; distinct tone sections:
- **plain** — neutral cleanup, the speaker's own voice (verbatim of master's spine).
- **casual** — light conversational, contractions, short sentences; "the way the speaker would type it in a quick message to a friend or teammate".
- **formal** — polished professional register, complete well-formed sentences; "suitable for professional or published use".
- **email** — warm-but-professional, short clear sentences with natural paragraph breaks; explicitly "do not add a subject line, greeting, signature, or any text the speaker did not say".

Full prose of each lives in the R5 subagent findings (retrievable from this ticket). Each is inserted as `messages[0]`; transcript stays `messages[1]`; same HTTP/parse/fallback plumbing.

## Fallback matrix

| Condition | Action |
|---|---|
| No key / empty key | skip call, insert raw text, Settings CTA "Add your Groq API key" |
| Offline/DNS/timeout | single retry after ~600 ms; then raw text |
| 429 | **no retry**, raw text + non-blocking Settings banner "Cleanup paused: Groq rate limit hit" (auto-resumes next clip once quota window passes); never spam mid-dictation |
| 5xx | single retry after ~1 s; then raw text + status.groq.com hint |
| other 4xx | no retry; 401/403 → "Invalid API key" hint; 404 model → re-probe /models |
| `finish_reason: length` | treat as failure, insert raw transcript (raise max_tokens next iteration) |
| parse failure | insert raw text |

Concurrency guard: ≤ ~8 in-flight; batch re-cleans paced ≤ 1 request/s.

## Free-tier math

~200-token clips, ~400-token static system prompt: 2 h/day ≈ 48K TPD (comfortable), **8 h/day ≈ 192K TPD — at the 200K ceiling**. Prompt caching on gpt-oss-120b cuts uncached cost to ~transcript+output (~400/call), making even heavy use comfortable. Requests are never the constraint. Escape hatches in order: prompt caching → second model pool (qwen3.6-27b) → Groq Developer plan (~10× limits).

## Sources

- Rate limits (org-level, cached-token exemption): https://console.groq.com/docs/rate-limits
- Models catalog: https://console.groq.com/docs/models · chat API: https://console.groq.com/docs/api-reference#chat-create · OpenAI compatibility: https://console.groq.com/docs/openai · errors: https://console.groq.com/docs/errors · prompt caching: https://console.groq.com/docs/prompt-caching · keys: https://console.groq.com/keys
- Catalog-rotation note: https://dev.to/build996/... ; transcript-cleanup prompting: https://theneuralbase.com/whisper/learn/advanced/post-processing-with-llm/ · https://metawhisp.com/blog/remove-filler-words-whisper-mac/ · https://github.com/danielrosehill/STT-Basic-Cleanup-System-Prompt · https://gist.github.com/briansunter/432e1db8746d0146623b7e4c744d9a0c