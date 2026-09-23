// Selection is an explicit set of IDs, never a live query that can grow after confirmation.
export function selectIds(selected:ReadonlySet<string>,ids:readonly string[],action:'add'|'remove'|'invert') {
  const next=new Set(selected);
  for(const id of ids){if(action==='remove'||(action==='invert'&&next.has(id)))next.delete(id);else next.add(id);}
  return next;
}
export function toggleId(selected:ReadonlySet<string>,id:string,orderedIds:readonly string[],anchor:string|null,range=false) {
  const start=anchor===null?-1:orderedIds.indexOf(anchor),end=orderedIds.indexOf(id);
  const ids=range&&start>=0&&end>=0?orderedIds.slice(Math.min(start,end),Math.max(start,end)+1):[id];
  return selectIds(selected,ids,selected.has(id)?'remove':'add');
}
export function selectionCounts(selected:ReadonlySet<string>,filteredIds:readonly string[],pageIds:readonly string[]) {
  const matched=filteredIds.filter(id=>selected.has(id)).length;
  return {total:selected.size,matched,hidden:selected.size-matched,pageCount:pageIds.filter(id=>selected.has(id)).length};
}
