// expect: react-hooks/immutability
export function Bad({ item }: { item: { value: string } }) {
  item.value = 'new';
  return <div>{item.value}</div>;
}
