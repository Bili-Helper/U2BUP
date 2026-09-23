export interface MediaInfo {duration:number|null; width:number;height:number;codec:string;aspect:string;signature:string;streams:Record<string,unknown>[]}
export interface Asset {id:string;relative_path:string;name:string;room_id:string;room_name:string;title:string;display_title:string|null;bytes:number;modified_ms:number;extension:string;role:string;started_at:string|null;time_source:string;metadata:MediaInfo|null;sidecars:string[];warnings:string[]}
export interface Room {id:string;name:string;aliases:string[];directories:string[];historical_title:string|null;online:Record<string,any>|null;refresh_error:string|null;refreshed_at:string|null}
export interface Output {id:string;name:string;room_id:string;room_name:string;title:string;aspect:string;duration:number;bytes:number;inputs:Asset[];reason:string}
export interface Plan {id:string;created_at:string;request:{asset_ids:string[];max_duration:number;max_bytes:number;max_gap:number;include_legacy:boolean};outputs:Output[];blocked:{asset_id:string;name:string;reason:string}[];estimated_bytes:number}
export interface Job {id:string;plan_id:string;status:string;created_at:string;updated_at:string;progress:number;message:string;completed_outputs:{output_id:string;path:string;bytes:number;duration:number;validation:string;source_ids:string[]}[]}
export interface Scan {running:boolean;completed:number;total:number;message:string}
export type TaskAction = 'cancel'|'pause'|'resume'|'retry';
export interface UnifiedTask {id:string;source_id:string;kind:'media'|'upload';status:string;title:string;created_at:string|null;updated_at:string|null;progress:number|null;message:string;capabilities:Record<TaskAction,boolean>;plan_id?:string;artifact_id?:string;video_id?:string;bytes?:number;offset?:number;completed_outputs?:Job['completed_outputs']}
export interface TaskSnapshot {tasks:UnifiedTask[];scan:Scan}
export interface Snapshot {library:{root:string;scanned_at:string|null;assets:Asset[];rooms:Room[];sidecar_count:number;errors:string[]};scan:Scan;jobs:Job[];plans:Plan[];settings:Record<string,string>}
