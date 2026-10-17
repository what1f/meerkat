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
