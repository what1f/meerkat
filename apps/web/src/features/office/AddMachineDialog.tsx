import { useState } from 'react';
import type * as DTO from '../../lib/api/generated';
import {
  Button,
  Command,
  Dialog,
  LinkButton,
  Reveal,
  Segmented,
  Skeleton,
  StatusDot,
  TextInput,
  useToast,
} from '../../design';
import {
  useEnableRemoteAccess,
  usePairingCode,
  useSetRemoteAccess,
} from '../../lib/queries/machines';
import { useT } from '../../lib/i18n';
import './messages';
import css from './AddMachineDialog.module.css';

type Session = {
  id: number;
  open: boolean;
  // Machines that were already here when the dialog opened; anything else is the new one.
  known: ReadonlySet<string> | null;
  access: DTO.RemoteAccess | null;
  invalid: boolean;
};
export type Pairing = ReturnType<typeof usePairing>;

export function usePairing() {
  const t = useT();
  const toast = useToast();
  const enable = useEnableRemoteAccess();
  const setRemote = useSetRemoteAccess();
  const [session, setSession] = useState<Session>({
    id: 0,
    open: false,
    known: null,
    access: null,
    invalid: false,
  });
  const code = usePairingCode(session.open && !!session.access?.address, session.id);
  const update = (id: number, next: Partial<Session>) =>
    setSession((s) => (s.id === id && s.open ? { ...s, ...next } : s));
  return {
    ...session,
    code: code.data ?? null,
    codeFailed: code.isError,
    pending: enable.isPending || setRemote.isPending,
    start: async (known: string[]) => {
      const id = session.id + 1;
      setSession({
        id,
        open: true,
        known: new Set(known),
        access: null,
        invalid: false,
      });
      try {
        update(id, { access: await enable.mutateAsync() });
      } catch {
        toast({ message: t('office.pairFailed') });
      }
    },
    apply: async (address: string) => {
      try {
        update(session.id, { access: await setRemote.mutateAsync({ address }), invalid: false });
      } catch {
        update(session.id, { invalid: true });
      }
    },
    close: () => setSession((s) => ({ ...s, open: false })),
  };
}

type PairPlatform = 'unix' | 'windows';

function defaultPairPlatform(): PairPlatform {
  return /Windows|Win32|Win64/i.test(`${navigator.userAgent} ${navigator.platform}`)
    ? 'windows'
    : 'unix';
}

export function AddMachineDialog({ pairing, connected }: { pairing: Pairing; connected?: string }) {
  const t = useT();
  const { access, code } = pairing;
  const [network, setNetwork] = useState(false);
  const [draft, setDraft] = useState<string | null>(null);
  const [platform, setPlatform] = useState<PairPlatform>(defaultPairPlatform);
  const saved = access?.address ?? '';
  const address = draft ?? saved;
  const edited = address.trim() !== '' && address.trim() !== saved;
  // Without a detected address there is nothing to pair against until one is entered.
  const unreachable = !!access && !access.address;
  const pair =
    access?.address && access.cert_sha256 && code
      ? `pair wss://${access.address} ${code.code} --cert-sha256 ${access.cert_sha256}`
      : null;
  const windowsScriptUrl = access?.install_script_url?.replace(/\/install\.sh$/, '/install.ps1');
  // Releases install the matching Machine first; development builds run the local one.
  const command =
    pair &&
    (access?.install_script_url
      ? platform === 'windows' && windowsScriptUrl
        ? `& ([scriptblock]::Create((irm ${windowsScriptUrl}))) ${pair}`
        : `curl -fsSL ${access.install_script_url} | sh -s -- ${pair}`
      : `meerkat-machine ${pair}`);
  const apply = () => {
    if (edited && !pairing.pending) void pairing.apply(address.trim());
  };
  return (
    <Dialog
      open={pairing.open}
      onClose={pairing.close}
      title={t('office.addTitle')}
      footer={
        <div className={css.foot}>
          <LinkButton disclosure open={network || unreachable} onClick={() => setNetwork(!network)}>
            {t('office.network')}
          </LinkButton>
          <Button variant={connected ? 'primary' : 'quiet'} onClick={pairing.close}>
            {connected ? t('office.done') : t('common.cancel')}
          </Button>
        </div>
      }
    >
      {!unreachable && (
        <div className={css.body}>
          <p className={css.hint}>
            {t(
              platform === 'windows' && access?.install_script_url
                ? 'office.runHintWindows'
                : 'office.runHint',
            )}
          </p>
          {access?.install_script_url && (
            <Segmented
              size="sm"
              aria-label={t('office.platform')}
              value={platform}
              onChange={setPlatform}
              options={[
                { value: 'unix' as const, label: t('office.platformUnix') },
                { value: 'windows' as const, label: t('office.platformWindows') },
              ]}
            />
          )}
          {command ? (
            <Command
              data-testid="pair-command"
              text={command}
              copyLabel={t('common.copy')}
              copiedLabel={t('common.copied')}
            />
          ) : pairing.codeFailed ? (
            <p className={css.hint}>{t('office.pairFailed')}</p>
          ) : (
            <Skeleton shape="line" width="100%" height={64} />
          )}
          <div className={css.state} data-testid="pair-state" data-connected={!!connected}>
            <StatusDot status={connected ? 'ok' : 'busy'} />
            {connected ?? t('office.waiting')}
          </div>
        </div>
      )}
      <Reveal open={network || unreachable}>
        <div className={css.network}>
          <TextInput
            data-testid="remote-address"
            size="sm"
            mono
            value={address}
            invalid={pairing.invalid}
            placeholder={t('office.addressPlaceholder')}
            onChange={setDraft}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.nativeEvent.isComposing) apply();
            }}
          />
          {edited && (
            <Button size="sm" loading={pairing.pending} onClick={apply}>
              {t('office.apply')}
            </Button>
          )}
        </div>
      </Reveal>
    </Dialog>
  );
}
