declare module '@aws-sdk/client-s3' {
  export class S3 {
    constructor(config?: any);
    send(command: any): Promise<any>;
  }
  export class PutObjectCommand {
    constructor(input: any);
  }
  export class GetObjectCommand {
    constructor(input: any);
  }
}
