import { useState } from 'react';
import type * as DTO from '../../lib/api/generated';
import {
  BrandMark,
  Button,
  Dialog,
  FormField,
  Icon,
  LinkButton,
  Select,
  Spinner,
  TextInput,
} from '../../design';
import { useLanguage, useT, type MessageKey } from '../../lib/i18n';
import { useBuiltinSignIn, useBuiltinSignOut } from '../../lib/queries/builtinAgent';
import { builtinAccount } from './account';
import { providers, regionOf } from './providers';
import { AccountLine } from './AccountLine';
import css from './BuiltinSignIn.module.css';

const reasons: Record<string, MessageKey> = {
  invalid_key: 'builtinAgent.invalidKey',
  unreachable: 'builtinAgent.unreachable',
};

// Signs this Computer's Built-in Agent in. Every member on the Computer shares the account.
export function BuiltinSignInDialog({
  machine,
  open,
  onClose,
  onSignedIn,
}: {
  machine: DTO.MachineCatalogItem;
  open: boolean;
  onClose: () => void;
  onSignedIn?: () => void;
}) {
  const t = useT();
  const language = useLanguage();
  const account = builtinAccount(machine);
  const [switching, setSwitching] = useState(false);
  const [choice, setChoice] = useState('chatgpt');
  const [key, setKey] = useState('');
  const [url, setUrl] = useState('');
  const reset = () => {
    setSwitching(false);
    setKey('');
  };
  const signIn = useBuiltinSignIn(() => {
    onSignedIn?.();
    reset();
    onClose();
  });
  const close = () => {
    signIn.cancel();
    reset();
    onClose();
  };
  const signOut = useBuiltinSignOut();
  const provider = providers.find((p) => p.id === choice);
  const browser = choice === 'chatgpt';
  const ready = browser || (key.trim() !== '' && (choice !== 'custom' || url.trim() !== ''));
  const reason = signIn.failed ? t(reasons[signIn.reason ?? ''] ?? 'builtinAgent.failed') : null;
  const submit = () =>
    signIn.start(
      browser
        ? { workspace: null, machine_id: machine.machine_id, method: 'chatgpt' }
        : {
            workspace: null,
            machine_id: machine.machine_id,
            method: 'api_key',
            base_url: provider ? regionOf(provider, language).base_url : url.trim(),
            api_key: key.trim(),
          },
    );
  const showAccount = !!account && !switching;
  return (
    <Dialog
      open={open}
      onClose={close}
      title={t('builtinAgent.dialogTitle').replace('{machine}', machine.display_name)}
      footer={
        showAccount ? (
          <div className={css.split}>
            <LinkButton onClick={() => signOut.mutate(machine.machine_id, { onSuccess: close })}>
              {t('builtinAgent.signOut')}
            </LinkButton>
            <Button onClick={() => setSwitching(true)}>{t('builtinAgent.switch')}</Button>
          </div>
        ) : (
          <div className={css.foot}>
            <span />
            <Button variant="quiet" onClick={close}>
              {t('builtinAgent.cancel')}
            </Button>
            <Button
              variant="primary"
              disabled={!ready}
              loading={signIn.waiting}
              leading={browser ? <Icon name="out" size={16} /> : undefined}
              onClick={submit}
            >
              {t(browser ? 'builtinAgent.openBrowser' : 'builtinAgent.connect')}
            </Button>
          </div>
        )
      }
    >
      {showAccount ? (
        <FormField label={t('builtinAgent.account')}>
          <AccountLine account={account} />
        </FormField>
      ) : (
        <div className={css.form}>
          <FormField label={t('builtinAgent.service')}>
            <Select
              variant="boxed"
              value={choice}
              onChange={(next) => {
                signIn.cancel();
                setChoice(next);
                setKey('');
              }}
              options={[
                {
                  value: 'chatgpt',
                  label: t('builtinAgent.chatgpt'),
                  leading: <BrandMark brand="openai" size={16} />,
                },
                ...providers.map((p) => ({
                  value: p.id,
                  label: p.name[language],
                  leading: <BrandMark brand={p.mark} size={16} />,
                })),
                {
                  value: 'custom',
                  label: t('builtinAgent.other'),
                  leading: <Icon name="more" size={16} />,
                },
              ]}
            />
          </FormField>
          {choice === 'custom' && (
            <FormField label={t('builtinAgent.baseUrl')}>
              <TextInput
                size="sm"
                mono
                value={url}
                onChange={setUrl}
                placeholder="https://"
                aria-label={t('builtinAgent.baseUrl')}
              />
            </FormField>
          )}
          {browser ? (
            (signIn.waiting || reason) && (
              <FormField label="" error={reason}>
                {signIn.waiting && (
                  <div className={css.line}>
                    <Spinner />
                    <span>{t('builtinAgent.waiting')}</span>
                  </div>
                )}
              </FormField>
            )
          ) : (
            <FormField
              label={t('builtinAgent.key')}
              error={reason}
              aside={
                provider && (
                  <a href={regionOf(provider, language).key_url} target="_blank" rel="noreferrer">
                    {t('builtinAgent.getKey')}
                    <Icon name="out" size={12} />
                  </a>
                )
              }
            >
              <TextInput
                size="sm"
                mono
                secret
                value={key}
                onChange={setKey}
                invalid={!!reason}
                placeholder={t('builtinAgent.pasteKey')}
                aria-label={t('builtinAgent.key')}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && !e.nativeEvent.isComposing && ready) submit();
                }}
              />
            </FormField>
          )}
        </div>
      )}
    </Dialog>
  );
}
