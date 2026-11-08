Patches applied on top of `../REVISION` by `../build`, in name order.

- `disable-response-language-check.patch`: turns off the reply-language check.
  It rejected replies whose script differed from the prompt's and failed the
  turn with `ResponseLanguageMismatch`. Meerkat prompts mix a Chinese envelope
  with ids such as `channel_id=group_chat`, so an English message could force a
  Chinese reply to fail. It also streamed its failure notice as assistant text.
- `neutral-oauth-page.patch`: the page the browser shows after a ChatGPT
  sign-in names Meerkat and carries no upstream logo, tab icon or footer link.
