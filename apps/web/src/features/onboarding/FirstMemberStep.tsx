import { InlineText, LinkButton, Reveal, Spinner, StepHeading } from '../../design';
import { Badge, Hang, QrTag, SoulSlot } from '../../scene';
import { useT } from '../../lib/i18n';
import { useState } from 'react';
import type { Flow } from './useOnboarding';
import css from './steps.module.css';

// The first member uses the same official registration as later members.
export function FirstMemberStep({ flow }: { flow: Flow }) {
  const t = useT();
  const [advanced, setAdvanced] = useState(false);
  const { draft, setDraft, creating, confirm, provider } = flow;
  const edit = (field: keyof typeof draft) => (value: string) =>
    setDraft((current) => ({ ...current, [field]: value }));
  return (
    <div className={css.step}>
      <StepHeading>{t('onboarding.firstMember')}</StepHeading>
      <div className={css.hero}>
        <Hang>
          <Badge
            size="intro"
            avatar={{ creating }}
            name={
              <InlineText
                data-testid="onboarding-member-name"
                mode="live"
                value={draft.name}
                onChange={edit('name')}
                placeholder={t('onboarding.name')}
                textStyle="heading"
                autoFocus
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && !e.nativeEvent.isComposing && flow.foot.ready)
                    flow.foot.next();
                }}
              />
            }
            role={
              <InlineText
                mode="live"
                value={draft.role}
                onChange={edit('role')}
                placeholder={t('onboarding.role')}
                textStyle="small"
              />
            }
          />
        </Hang>
        {confirm && !flow.permissions && (
          <QrTag
            data-testid="onboarding-confirm"
            state="wait"
            value={confirm}
            brand={provider}
            onRefresh={flow.refreshQr}
            refreshLabel={t('onboarding.refresh')}
          />
        )}
      </div>
      <div className={css.under}>
        {flow.permissions && confirm ? (
          <>
            <Spinner />
            <LinkButton
              data-testid="onboarding-permissions"
              onClick={() => window.open(confirm, '_blank', 'noopener')}
            >
              {t('onboarding.confirmPermissions')}
            </LinkButton>
            <span>{t('onboarding.waiting')}</span>
          </>
        ) : confirm ? (
          <>
            <Spinner />
            <span>
              {t(provider === 'feishu' ? 'onboarding.confirmFeishu' : 'onboarding.confirmTelegram')}
            </span>
          </>
        ) : flow.failed ? (
          <span>{t('onboarding.connectFailed')}</span>
        ) : (
          !creating && (
            <LinkButton disclosure open={advanced} onClick={() => setAdvanced(!advanced)}>
              {t('onboarding.advanced')}
            </LinkButton>
          )
        )}
      </div>
      <span className={css.advanced}>
        <Reveal open={advanced && !creating}>
          <SoulSlot
            value={draft.soul}
            onChange={edit('soul')}
            label={t('onboarding.soul')}
            placeholder={t('onboarding.soulPlaceholder')}
            onFocusChange={() => undefined}
          />
        </Reveal>
      </span>
    </div>
  );
}
