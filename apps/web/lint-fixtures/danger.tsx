// expect: react/no-danger
export const Bad = () => <div dangerouslySetInnerHTML={{ __html: 'text' }} />;
