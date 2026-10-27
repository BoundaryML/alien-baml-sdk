// The same fully qualified name as a function of the Alien project
// (user.setup.cloudformation_launch_url), with a different body. Two unrelated
// projects can easily share a name like this.
class CloudFormationLaunch {
    templateUrl: string,
    region: string,
    stackName: string,
    parameters: map<string, string>,
}

function cloudformation_launch_url(input: CloudFormationLaunch) -> string {
    "not an AWS link: the other project's cloudformation_launch_url ran"
}
