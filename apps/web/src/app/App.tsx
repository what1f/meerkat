import { lazy, Suspense } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { ToastProvider } from '../design';
import { I18nProvider, useLanguage } from '../lib/i18n';
import { ThemeProvider } from '../lib/theme';
import { AppShell } from '../shell';
import { usePage } from './routes';
const queryClient = new QueryClient();
const DesignPreview = import.meta.env.DEV ? lazy(() => import('../dev/DesignPreview')) : null;
export function App() {
  const page = usePage();
  useLanguage();
  return (
    <QueryClientProvider client={queryClient}>
      <I18nProvider>
        <ThemeProvider>
          <ToastProvider>
            {import.meta.env.DEV && page.kind === 'design' && DesignPreview ? (
              <Suspense>
                <DesignPreview />
              </Suspense>
            ) : (
              <AppShell sidebar={null} topBar={null}>
                {null}
              </AppShell>
            )}
          </ToastProvider>
        </ThemeProvider>
      </I18nProvider>
    </QueryClientProvider>
  );
}
