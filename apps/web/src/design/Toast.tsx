import type { ReactNode } from 'react';
import type { UIProps } from './types';
import { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react';
import { Button } from './Button';
import css from './Toast.module.css';
export type ToastProviderProps = Omit<UIProps, 'className' | 'style'> & { children: ReactNode };
export type ToastOptions = {
  leading?: ReactNode;
  message: string;
  action?: { label: string; onAction: () => void };
};
const ToastContext = createContext<(opts: ToastOptions) => void>(() => {});
export function useToast() {
  return useContext(ToastContext);
}
export function ToastProvider({ children, ...rest }: ToastProviderProps) {
  const [toasts, setToasts] = useState<(ToastOptions & { id: number })[]>([]);
  const sequence = useRef(0);
  const timers = useRef(new Set<ReturnType<typeof setTimeout>>());
  useEffect(() => {
    const active = timers.current;
    return () => {
      active.forEach(clearTimeout);
    };
  }, []);
  const show = useCallback((opts: ToastOptions) => {
    const id = ++sequence.current;
    setToasts((items) => [...items, { ...opts, id }]);
    const timer = setTimeout(() => {
      setToasts((items) => items.filter((item) => item.id !== id));
      timers.current.delete(timer);
    }, 3600);
    timers.current.add(timer);
  }, []);
  return (
    <ToastContext value={show}>
      {children}
      <div {...rest} className={css.root} role="region" aria-live="polite">
        {toasts.map((toast) => (
          <div key={toast.id} role="status">
            {toast.leading}
            {toast.message}
            {toast.action && (
              <Button
                onClick={() => {
                  toast.action?.onAction();
                  setToasts((items) => items.filter((item) => item.id !== toast.id));
                }}
              >
                {toast.action.label}
              </Button>
            )}
          </div>
        ))}
      </div>
    </ToastContext>
  );
}
