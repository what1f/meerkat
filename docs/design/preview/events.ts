// Only sample-data changes; the design never connects to a Runtime.
import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
export function useManagementEvents(_workspace?: string) {
  "use no memo";
  const client = useQueryClient();
  useEffect(() => {
    const refresh = () => {
      void client.invalidateQueries();
    };
    addEventListener("meerkat-design-data", refresh);
    return () => removeEventListener("meerkat-design-data", refresh);
  }, [client]);
}
