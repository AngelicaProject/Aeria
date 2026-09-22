type DocumentTabsProps = {
  label: string;
  detail?: string;
};

export function DocumentTabs({ label, detail }: DocumentTabsProps) {
  return <div className="document-tabs"><button className="document-tab active" type="button" aria-current="page"><span>{label}</span>{detail ? <small>{detail}</small> : null}</button></div>;
}
