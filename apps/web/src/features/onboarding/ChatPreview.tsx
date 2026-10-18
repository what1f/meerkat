import { Message, Spinner } from '../../design';
import css from './ChatPreview.module.css';

type Line = { from: 'me' | 'them' | 'done'; text: string; delay?: number };

// A short chat beside the desk: what you sent and what the member answered.
export function ChatPreview({
  lines,
  waiting,
  avatar,
}: {
  lines: Line[];
  waiting?: string;
  avatar?: string;
}) {
  return (
    <div className={css.root} data-testid="onboarding-chat">
      {waiting ? (
        <span className={css.wait}>
          <Spinner />
          <span>{waiting}</span>
        </span>
      ) : (
        lines.map((line, i) => (
          <Message
            key={i}
            from={line.from === 'me' ? 'me' : 'them'}
            variant={line.from === 'done' ? 'done' : 'bubble'}
            avatar={line.from === 'them' ? avatar : undefined}
            delay={line.delay}
          >
            {line.text}
          </Message>
        ))
      )}
    </div>
  );
}
