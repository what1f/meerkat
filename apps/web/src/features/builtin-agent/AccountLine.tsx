import { BrandMark, Icon } from '../../design';
import { useLanguage, useT } from '../../lib/i18n';
import type { builtinAccount } from './account';
import { providerOfUrl } from './providers';
import css from './BuiltinSignIn.module.css';

// The account this Computer is signed in with: who for ChatGPT, which service for a key.
export function AccountLine({
  account,
}: {
  account: NonNullable<ReturnType<typeof builtinAccount>>;
}) {
  const t = useT();
  const language = useLanguage();
  const provider = account.method === 'api_key' ? providerOfUrl(account.label) : undefined;
  return (
    <div className={css.line}>
      {account.method === 'chatgpt' ? (
        <BrandMark brand="openai" size={16} />
      ) : provider ? (
        <BrandMark brand={provider.mark} size={16} />
      ) : (
        <Icon name="key" size={16} />
      )}
      <span>
        {account.method === 'chatgpt'
          ? (account.label ?? t('builtinAgent.chatgpt'))
          : (provider?.name[language] ?? account.label ?? t('builtinAgent.other'))}
      </span>
    </div>
  );
}
