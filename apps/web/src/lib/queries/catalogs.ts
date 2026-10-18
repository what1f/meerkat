import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import type * as DTO from '../api/generated';
import { call } from '../api/client';
import { keys } from './keys';
export function useProviderCatalog(input: DTO.CatalogRequest, enabled = true) {
  return useQuery({
    queryKey: [...keys.catalogs, 'get_provider_catalog', { ...input, request_id: undefined }],
    queryFn: ({ signal }) => call('get_provider_catalog', input, signal),
    enabled,
  });
}
export function useHarnessCatalog(input: DTO.CatalogRequest, enabled = true) {
  return useQuery({
    queryKey: [...keys.catalogs, 'get_harness_catalog', { ...input, request_id: undefined }],
    queryFn: ({ signal }) => call('get_harness_catalog', input, signal),
    enabled,
  });
}
export function useModelCatalog(input: DTO.ModelCatalogRequest, enabled = true) {
  return useQuery({
    queryKey: [...keys.catalogs, 'get_model_catalog', { ...input, request_id: undefined }],
    queryFn: ({ signal }) => call('get_model_catalog', input, signal),
    enabled,
  });
}
export function useReactionCatalog(input: DTO.CatalogRequest, enabled = true) {
  return useQuery({
    queryKey: [...keys.catalogs, 'get_reaction_catalog', { ...input, request_id: undefined }],
    queryFn: ({ signal }) => call('get_reaction_catalog', input, signal),
    enabled,
  });
}
export function useListProviderInterfaces(
  input: DTO.ListProviderInterfacesRequest,
  enabled = true,
) {
  return useQuery({
    queryKey: [...keys.catalogs, 'list_provider_interfaces', { ...input, request_id: undefined }],
    queryFn: ({ signal }) => call('list_provider_interfaces', input, signal),
    enabled,
  });
}
export function useInvokeProviderInterface() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: DTO.InvokeProviderInterfaceRequest) =>
      call('invoke_provider_interface', input),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.catalogs });
    },
  });
}

export function useFigurePacks(input: DTO.ListFigurePacksRequest, enabled = true) {
  return useQuery({
    queryKey: [...keys.figures, input],
    queryFn: ({ signal }) => call('list_figure_packs', input, signal),
    enabled,
  });
}
export function useImportFigurePack() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: async (input: { workspace: DTO.WorkspaceRef | null; file: File }) => {
      const bytes = new Uint8Array(await input.file.arrayBuffer());
      const chunks: string[] = [];
      for (let offset = 0; offset < bytes.length; offset += 8192) {
        chunks.push(String.fromCharCode(...bytes.subarray(offset, offset + 8192)));
      }
      return call('import_figure_pack', {
        workspace: input.workspace,
        bytes: btoa(chunks.join('')),
      });
    },
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.figures });
    },
  });
}
