// Command genops writes alien/operations.go (gitignored): one method per
// generated Platform and Manager operation, grouped as in Alien's TypeScript
// SDK (c.Deployments.List(ctx, params)); an operation without a group is a
// method of the client itself (c.AcceptWorkspaceInvitation(ctx, token)). It
// reads the generated baml_sdk/functions.go, so it holds no per-operation
// code; scripts/generate.sh runs it after baml generate.
//
// It also writes alien/operations_table_test.go (gitignored): the
// conformance harness's table of every operation (`call.op` in a scenario,
// conformance/README.md), each calling its method.
//
// It works in three steps: parseOperations reads the generated functions,
// declarations names everything operations.go declares, and the templates at
// the end of this file print both files.
//
//	go run ./internal/genops    (from sdk/go)
package main

import (
	"bytes"
	"fmt"
	"go/ast"
	"go/format"
	"go/parser"
	"go/token"
	"maps"
	"os"
	"regexp"
	"slices"
	"strconv"
	"strings"
	"text/template"
	"unicode"
)

const (
	input     = "baml_sdk/functions.go"
	output    = "alien/operations.go"
	tableFile = "alien/operations_table_test.go"

	// failureLine starts the generated doc sentence about runtime errors,
	// which methods drop: they return *AlienError.
	failureLine = "BAML failures are returned as Go errors"
)

// api is one of the two generated APIs.
type api struct {
	path       string   // BAML namespace: "platform"
	clientType string   // generated client type
	facade     string   // the facade client type, in alien.go
	fields     []string // its hand-written fields and methods, which no group or method may shadow
	clientExpr string   // the generated client inside a service method
	apiType    string   // the struct of groups embedded in the facade client
	service    string   // prefix of service and params type names
	newFacade  string   // the test harness's facade client for a scenario's args
}

// The Manager facade's generated client is ManagerClient.Generated, not
// .Manager: the Manager API has a group named manager (m.Manager.ManagerInfo).
var apis = []*api{
	{
		path:       "platform",
		clientType: "PlatformPlatformClient",
		facade:     "Client",
		fields:     []string{"Platform", "Setup", "PlatformAPI", "Format", "LogValue"},
		clientExpr: "s.client.Platform",
		apiType:    "PlatformAPI",
		service:    "",
		newFacade:  "facade",
	},
	{
		path:       "manager",
		clientType: "ManagerManagerClient",
		facade:     "ManagerClient",
		fields:     []string{"Generated", "ManagerAPI"},
		clientExpr: "s.client.Generated",
		apiType:    "ManagerAPI",
		service:    "Manager",
		newFacade:  "managerFacade",
	},
}

// param is a required argument (name is the generated Go name) or an
// optional one (option is the With… function); wire is its BAML name.
type param struct {
	name   string
	wire   string
	typ    ast.Expr
	doc    string
	option string
}

type operation struct {
	api        *api
	function   string // generated function name
	group      string // "" for an operation without a group
	method     string
	opID       string
	doc        []string
	argDocs    map[string]string
	optionType string // the type of the generated function's variadic options
	required   []param
	optional   []param
	result     ast.Expr // nil when the operation returns only an error
}

var (
	opIDPattern = regexp.MustCompile("\\(operation `([A-Za-z0-9_]+)`\\)")
	argDocLine  = regexp.MustCompile("^- `([A-Za-z0-9_]+)`: ?(.*)$")
	initialisms = map[string]string{"api": "API", "url": "URL", "urls": "URLs", "id": "ID", "ids": "IDs", "http": "HTTP", "json": "JSON", "aws": "AWS", "gcp": "GCP", "uri": "URI", "uuid": "UUID", "ip": "IP", "dns": "DNS", "tls": "TLS", "ssh": "SSH", "sql": "SQL", "cpu": "CPU", "ui": "UI", "ttl": "TTL", "oauth": "OAuth", "xks": "XKS"}
	predeclared = map[string]bool{"string": true, "bool": true, "int": true, "int64": true, "float64": true, "any": true, "error": true, "byte": true}
)

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, "genops:", err)
		os.Exit(1)
	}
}

func run() error {
	ops, err := parseOperations(input)
	if err != nil {
		return err
	}
	decls, err := declarations(ops)
	if err != nil {
		return err
	}
	if err := writeGo(output, operationsTemplate, decls); err != nil {
		return err
	}
	return writeGo(tableFile, tableTemplate, tableRows(ops))
}

// writeGo writes what tmpl prints for data to path, gofmt'ed.
func writeGo(path string, tmpl *template.Template, data any) error {
	var source bytes.Buffer
	if err := tmpl.Execute(&source, data); err != nil {
		return fmt.Errorf("%s: %w", path, err)
	}
	formatted, err := format.Source(source.Bytes())
	if err != nil {
		return fmt.Errorf("gofmt %s: %w\n%s", path, err, source.Bytes())
	}
	return os.WriteFile(path, formatted, 0o644)
}

// ---------------------------------------------------------------------------
// Reading the generated functions.

// parseOperations reads every Platform and Manager operation in the generated
// functions file, with its optional arguments.
func parseOperations(path string) ([]*operation, error) {
	file, err := parser.ParseFile(token.NewFileSet(), path, nil, parser.ParseComments)
	if err != nil {
		return nil, err
	}
	options := map[string][]*ast.FuncDecl{} // option type -> With… functions
	var ops []*operation
	for _, decl := range file.Decls {
		fn, ok := decl.(*ast.FuncDecl)
		if !ok || fn.Recv != nil || !fn.Name.IsExported() {
			continue
		}
		if strings.HasPrefix(fn.Name.Name, "With") && fn.Type.Results != nil && len(fn.Type.Results.List) == 1 {
			if id, ok := fn.Type.Results.List[0].Type.(*ast.Ident); ok {
				options[id.Name] = append(options[id.Name], fn)
			}
			continue
		}
		op, err := parseOperation(fn)
		if err != nil {
			return nil, fmt.Errorf("%s: %w", fn.Name.Name, err)
		}
		if op != nil {
			ops = append(ops, op)
		}
	}
	if len(ops) == 0 {
		return nil, fmt.Errorf("no operations in %s", path)
	}
	for _, op := range ops {
		if err := op.attachOptions(options); err != nil {
			return nil, fmt.Errorf("%s: %w", op.function, err)
		}
	}
	return ops, nil
}

// parseOperation returns nil for a function that is not a Platform or Manager
// operation (helpers, setup, compat). Operations are user.<api>.<group>.<name>,
// or user.<api>.<name> without a group.
func parseOperation(fn *ast.FuncDecl) (*operation, error) {
	parts := strings.Split(stringArg(fn.Body, "CallWithPreparedOptions", 1), ".")
	if len(parts) == 3 {
		parts = []string{parts[0], parts[1], "", parts[2]}
	}
	if len(parts) != 4 || parts[0] != "user" {
		return nil, nil
	}
	i := slices.IndexFunc(apis, func(a *api) bool { return a.path == parts[1] })
	if i < 0 {
		return nil, nil
	}
	a := apis[i]
	params := fn.Type.Params.List
	if len(params) < 3 || typeString(params[1].Type) != "baml_sdk."+a.clientType {
		return nil, fmt.Errorf("want (ctx, client, ..., options...), got %d parameters", len(params))
	}
	op := &operation{api: a, function: fn.Name.Name, group: parts[2], method: parts[3]}
	if fn.Doc == nil {
		return nil, fmt.Errorf("no doc comment")
	}
	op.doc, op.argDocs = splitDoc(fn.Doc.Text(), fn.Name.Name, goName(parts[3]))
	m := opIDPattern.FindStringSubmatch(strings.Join(op.doc, "\n"))
	if m == nil {
		return nil, fmt.Errorf("no operation ID in its doc comment")
	}
	op.opID = m[1]
	wires := argumentNames(fn)
	for _, field := range params[2 : len(params)-1] {
		for _, name := range field.Names {
			wire, ok := wires[name.Name]
			if !ok {
				return nil, fmt.Errorf("parameter %s is not in the arguments_ map", name.Name)
			}
			op.required = append(op.required, param{name: name.Name, wire: wire, typ: field.Type})
		}
	}
	ellipsis, ok := params[len(params)-1].Type.(*ast.Ellipsis)
	if !ok {
		return nil, fmt.Errorf("last parameter is not variadic")
	}
	op.optionType = strings.TrimPrefix(typeString(ellipsis.Elt), "baml_sdk.")
	switch results := fn.Type.Results.List; len(results) {
	case 1:
	case 2:
		op.result = results[0].Type
	default:
		return nil, fmt.Errorf("unexpected results")
	}
	return op, nil
}

// attachOptions sets the operation's optional arguments from its With…
// functions, in declaration order.
func (op *operation) attachOptions(options map[string][]*ast.FuncDecl) error {
	if op.optionType == "baml_go.CallOption" {
		return nil
	}
	for _, fn := range options[op.optionType] {
		wire := stringArg(fn.Body, "WithArgument", 0)
		if wire == "" {
			return fmt.Errorf("%s: no WithArgument name", fn.Name.Name)
		}
		star, ok := fn.Type.Params.List[0].Type.(*ast.StarExpr)
		if !ok {
			return fmt.Errorf("%s does not take a pointer", fn.Name.Name)
		}
		op.optional = append(op.optional, param{wire: wire, typ: star.X, doc: op.argDocs[wire], option: fn.Name.Name})
	}
	if len(op.optional) == 0 {
		return fmt.Errorf("no With… functions for %s", op.optionType)
	}
	return nil
}

// stringArg returns the string literal passed as argument i of the first
// call to a function or method named name in node, or "" when there is none:
// the BAML path in baml_go.CallWithPreparedOptions(ctx, "user.platform.…"),
// the BAML name in WithArgument("limit", …).
func stringArg(node ast.Node, name string, i int) string {
	value := ""
	ast.Inspect(node, func(n ast.Node) bool {
		call, ok := n.(*ast.CallExpr)
		if !ok || value != "" || len(call.Args) <= i {
			return value == ""
		}
		if sel, ok := call.Fun.(*ast.SelectorExpr); ok && sel.Sel.Name == name {
			if lit, ok := call.Args[i].(*ast.BasicLit); ok {
				value, _ = strconv.Unquote(lit.Value)
			}
		}
		return value == ""
	})
	return value
}

// argumentNames maps each Go parameter of a generated function to its BAML
// name, from the function's arguments_ map ("command_id": baml_go.String(commandId)).
func argumentNames(fn *ast.FuncDecl) map[string]string {
	out := map[string]string{}
	ast.Inspect(fn.Body, func(n ast.Node) bool {
		kv, ok := n.(*ast.KeyValueExpr)
		if !ok {
			return true
		}
		key, ok := kv.Key.(*ast.BasicLit)
		if !ok || key.Kind != token.STRING {
			return true
		}
		wire, err := strconv.Unquote(key.Value)
		if err != nil {
			return true
		}
		ast.Inspect(kv.Value, func(n ast.Node) bool {
			if id, ok := n.(*ast.Ident); ok {
				if _, seen := out[id.Name]; !seen {
					out[id.Name] = wire
				}
			}
			return true
		})
		return false
	})
	return out
}

// splitDoc returns the operation's doc lines, without the runtime-error
// sentence (methods return *AlienError) and with the method name in place of
// the generated one, and each argument's description.
func splitDoc(text, name, method string) ([]string, map[string]string) {
	var lines []string
	args := map[string]string{}
	current := "" // the argument whose description continues on indented lines
	for _, line := range strings.Split(strings.TrimRight(text, "\n"), "\n") {
		if strings.HasPrefix(line, failureLine) {
			continue
		}
		lines = append(lines, line)
		switch m := argDocLine.FindStringSubmatch(line); {
		case m != nil:
			current = m[1]
			args[current] = m[2]
		case current != "" && strings.HasPrefix(line, "\t"):
			args[current] += "\n\n" + strings.TrimSpace(line)
		case line != "":
			current = ""
		}
	}
	for len(lines) > 0 && lines[len(lines)-1] == "" {
		lines = lines[:len(lines)-1]
	}
	if len(lines) > 0 {
		lines[0] = method + strings.TrimPrefix(lines[0], name)
	}
	return lines, args
}

// ---------------------------------------------------------------------------
// Go names and types.

// goName turns a camelCase or snake_case BAML name into an exported Go name
// with Go's initialisms: getTemplateUrls -> GetTemplateURLs.
func goName(s string) string {
	var b strings.Builder
	for _, w := range words(s) {
		if special, ok := initialisms[strings.ToLower(w)]; ok {
			b.WriteString(special)
		} else {
			b.WriteString(strings.ToUpper(w[:1]) + w[1:])
		}
	}
	return b.String()
}

// paramName is goName for an unexported name: externalId -> externalID.
func paramName(s string) string {
	ws := words(s)
	return strings.ToLower(ws[0]) + goName(strings.Join(ws[1:], "_"))
}

// words splits a name at underscores and where a camelCase word starts:
// getTemplateURLs -> get Template URLs.
func words(s string) []string {
	var out []string
	for _, part := range strings.Split(s, "_") {
		runes := []rune(part)
		start := 0
		for i := 1; i < len(runes); i++ {
			if startsWord(runes, i) {
				out = append(out, string(runes[start:i]))
				start = i
			}
		}
		if start < len(runes) {
			out = append(out, string(runes[start:]))
		}
	}
	return out
}

// startsWord reports whether runes[i] starts a word: an upper-case letter
// after a lower-case letter or a digit (getTemplate), or the last upper-case
// letter of an initialism that a lower-case letter follows (URLList).
func startsWord(runes []rune, i int) bool {
	prev := runes[i-1]
	switch {
	case !unicode.IsUpper(runes[i]):
		return false
	case unicode.IsLower(prev), unicode.IsDigit(prev):
		return true
	default:
		return unicode.IsUpper(prev) && i+1 < len(runes) && unicode.IsLower(runes[i+1])
	}
}

// typeString prints a type of the generated package, qualified for package alien.
func typeString(e ast.Expr) string {
	switch t := e.(type) {
	case *ast.Ident:
		if predeclared[t.Name] || !ast.IsExported(t.Name) {
			return t.Name
		}
		return "baml_sdk." + t.Name
	case *ast.StarExpr:
		return "*" + typeString(t.X)
	case *ast.ArrayType:
		return "[]" + typeString(t.Elt)
	case *ast.MapType:
		return fmt.Sprintf("map[%s]%s", typeString(t.Key), typeString(t.Value))
	case *ast.SelectorExpr:
		return typeString(t.X) + "." + t.Sel.Name
	}
	panic(fmt.Sprintf("unsupported type %T", e))
}

// fieldType: an optional argument's params field. Lists are slices and
// everything else is a pointer, so nil means unset and a zero value (an empty
// string, 0) is still sent, as with the generated With… options.
func fieldType(t ast.Expr) string {
	s := typeString(t)
	if strings.HasPrefix(s, "[]") {
		return s
	}
	return "*" + s
}

// serviceType is the type whose methods are a group's operations; operations
// without a group are methods of the API struct itself.
func (a *api) serviceType(group string) string {
	if group == "" {
		return a.apiType
	}
	return a.service + goName(group) + "Service"
}

// paramsType: Platform ListDeploymentsParams, Manager ManagerListDeploymentsParams.
func (op *operation) paramsType() string { return op.api.service + goName(op.opID) + "Params" }

// onlyBody: the single optional argument is the request body, taken as a
// trailing pointer argument as in Alien's TypeScript SDK (createToken(id, body)).
func (op *operation) onlyBody() bool { return len(op.optional) == 1 && op.optional[0].wire == "body" }

// callName is the operation as Alien's TypeScript SDK names it: deployments.list,
// or acceptWorkspaceInvitation without a group.
func (op *operation) callName() string {
	if op.group == "" {
		return op.method
	}
	return op.group + "." + op.method
}

// ---------------------------------------------------------------------------
// What operations.go declares, for operationsTemplate.

// apiDecl is an API's struct of groups (PlatformAPI), its own methods (the
// operations without a group) and its groups.
type apiDecl struct {
	Name    string // Platform
	Type    string // PlatformAPI
	Facade  string // the facade client that embeds it: Client
	Methods []methodDecl
	Groups  []groupDecl
}

// groupDecl is a group's field in the API struct, its service type and the
// service's methods: Deployments *DeploymentsService.
type groupDecl struct {
	Name    string // BAML name: deploymentGroups
	Field   string // DeploymentGroups
	Service string // DeploymentGroupsService
	Methods []methodDecl
}

// methodDecl is an operation's method and, when the operation has optional
// arguments other than a lone body, their params struct.
type methodDecl struct {
	Doc      []string
	Receiver string
	Name     string
	Params   string // ctx context.Context, id string, params *GetDeploymentParams
	Results  string // error, or (T, error)
	Wrap     string // Err, or Do when there is a result
	Function string // the generated function it calls
	Args     string // ctx, s.client.Platform, id, params.options()...
	Options  *paramsDecl
}

// paramsDecl is a params struct and its options method, which turns the set
// fields into the generated function's With… options.
type paramsDecl struct {
	Type     string // GetDeploymentParams
	CallName string // deployments.get
	Function string
	Fields   []fieldDecl
}

type fieldDecl struct {
	Doc    []string
	Name   string // Include
	Type   string // a pointer or a slice, so that nil is unset
	Option string // its With… function
	Arg    string // the With… function's argument: p.Include, or &p.Include for a slice
}

// typeNames holds the types operations.go declares, by name, to catch two
// that would get the same one.
type typeNames map[string]string

func (t typeNames) claim(name, what string) error {
	if prev, ok := t[name]; ok {
		return fmt.Errorf("%s and %s are both named %s", prev, what, name)
	}
	t[name] = what
	return nil
}

// declarations groups the operations by API and group, sorts them by name,
// and names everything operations.go declares.
func declarations(ops []*operation) ([]apiDecl, error) {
	types := typeNames{}
	var decls []apiDecl
	for _, a := range apis {
		byGroup := map[string][]*operation{} // "" holds the operations without a group
		for _, op := range ops {
			if op.api == a {
				byGroup[op.group] = append(byGroup[op.group], op)
			}
		}
		if err := types.claim(a.apiType, a.path+" API"); err != nil {
			return nil, err
		}
		if err := a.checkMembers(byGroup); err != nil {
			return nil, err
		}
		decl := apiDecl{Name: goName(a.path), Type: a.apiType, Facade: a.facade}
		for _, group := range slices.Sorted(maps.Keys(byGroup)) {
			service := a.serviceType(group)
			if group != "" {
				if err := types.claim(service, a.path+"."+group); err != nil {
					return nil, err
				}
			}
			methods, err := methodDecls(byGroup[group], types)
			if err != nil {
				return nil, err
			}
			if group == "" {
				decl.Methods = methods
			} else {
				decl.Groups = append(decl.Groups, groupDecl{Name: group, Field: goName(group), Service: service, Methods: methods})
			}
		}
		decls = append(decls, decl)
	}
	return decls, nil
}

// checkMembers fails when a group and an operation without a group get the
// same Go name, or when either would shadow a field or method of the facade.
func (a *api) checkMembers(byGroup map[string][]*operation) error {
	members := map[string]string{} // Go name -> the group or operation it is
	for group := range byGroup {
		if group != "" {
			members[goName(group)] = "group " + a.path + "." + group
		}
	}
	for _, op := range byGroup[""] {
		name := goName(op.method)
		if prev, ok := members[name]; ok {
			return fmt.Errorf("%s and operation %s.%s are both named %s", prev, a.path, op.method, name)
		}
		members[name] = "operation " + a.path + "." + op.method
	}
	for name, what := range members {
		if slices.Contains(a.fields, name) {
			return fmt.Errorf("%s would shadow %s.%s", what, a.facade, name)
		}
	}
	return nil
}

// methodDecls returns the methods of one group's operations, in name order.
func methodDecls(ops []*operation, types typeNames) ([]methodDecl, error) {
	slices.SortFunc(ops, func(x, y *operation) int { return strings.Compare(x.method, y.method) })
	var methods []methodDecl
	for _, op := range ops {
		m := op.methodDecl()
		if m.Options != nil {
			if err := types.claim(m.Options.Type, op.opID+" parameters"); err != nil {
				return nil, err
			}
		}
		methods = append(methods, m)
	}
	return methods, nil
}

func (op *operation) methodDecl() methodDecl {
	m := methodDecl{
		Doc:      op.doc,
		Receiver: op.api.serviceType(op.group),
		Name:     goName(op.method),
		Results:  "error",
		Wrap:     "Err",
		Function: op.function,
	}
	params := []string{"ctx context.Context"}
	args := []string{"ctx", op.api.clientExpr}
	for _, p := range op.required {
		params = append(params, paramName(p.name)+" "+typeString(p.typ))
		args = append(args, paramName(p.name))
	}
	switch {
	case op.onlyBody():
		body := op.optional[0]
		params = append(params, "body *"+typeString(body.typ))
		args = append(args, fmt.Sprintf("baml_sdk.%s(body)", body.option))
	case len(op.optional) > 0:
		params = append(params, "params *"+op.paramsType())
		args = append(args, "params.options()...")
		m.Options = op.paramsDecl()
	}
	if op.result != nil {
		m.Results, m.Wrap = fmt.Sprintf("(%s, error)", typeString(op.result)), "Do"
	}
	m.Params, m.Args = strings.Join(params, ", "), strings.Join(args, ", ")
	return m
}

func (op *operation) paramsDecl() *paramsDecl {
	decl := &paramsDecl{Type: op.paramsType(), CallName: op.callName(), Function: op.function}
	for _, p := range op.optional {
		field := fieldDecl{Name: goName(p.wire), Type: fieldType(p.typ), Option: p.option}
		if p.doc != "" {
			field.Doc = strings.Split(p.doc, "\n")
		}
		field.Arg = "p." + field.Name
		if !strings.HasPrefix(field.Type, "*") {
			field.Arg = "&" + field.Arg
		}
		decl.Fields = append(decl.Fields, field)
	}
	return decl
}

// ---------------------------------------------------------------------------
// The conformance harness's operation table, for tableTemplate.

// tableRow is one operation's entry: it calls the operation's method with the
// scenario's arguments, decoded by the SDK (arg, optArg and optList in
// operations_test.go), and returns its result as result does.
type tableRow struct {
	Key    string // the operation as a scenario's call.op names it: platform.deployments.list
	Call   string // facade(a).Deployments.List(ctx, &ListDeploymentsParams{…})
	Result bool   // false when the method returns only an error
}

func tableRows(ops []*operation) []tableRow {
	var rows []tableRow
	for _, op := range ops {
		rows = append(rows, tableRow{Key: op.api.path + "." + op.callName(), Call: op.tableCall(), Result: op.result != nil})
	}
	slices.SortFunc(rows, func(x, y tableRow) int { return strings.Compare(x.Key, y.Key) })
	return rows
}

func (op *operation) tableCall() string {
	target := op.api.newFacade + "(a)"
	if op.group != "" {
		target += "." + goName(op.group)
	}
	args := []string{"ctx"}
	for _, p := range op.required {
		args = append(args, fmt.Sprintf("arg[%s](ctx, a, %q)", typeString(p.typ), p.wire))
	}
	switch {
	case op.onlyBody():
		args = append(args, fmt.Sprintf(`optArg[%s](ctx, a, "body")`, typeString(op.optional[0].typ)))
	case len(op.optional) > 0:
		var fields []string
		for _, p := range op.optional {
			helper, elem := "optArg", typeString(p.typ)
			if list, ok := strings.CutPrefix(elem, "[]"); ok {
				helper, elem = "optList", list
			}
			fields = append(fields, fmt.Sprintf("%s: %s[%s](ctx, a, %q),", goName(p.wire), helper, elem, p.wire))
		}
		args = append(args, fmt.Sprintf("&%s{\n%s\n}", op.paramsType(), strings.Join(fields, "\n")))
	}
	return fmt.Sprintf("%s.%s(%s)", target, goName(op.method), strings.Join(args, ", "))
}

// ---------------------------------------------------------------------------
// The two files. gofmt aligns and indents what they print.

var operationsTemplate = template.Must(template.New(output).Parse(`
// Code generated by internal/genops from baml_sdk/functions.go. DO NOT EDIT.

package alien

import (
	"context"

	"example.com/alien-baml-sdk/go/baml_sdk"
)

{{- range $api := .}}

// {{.Type}} holds the {{.Name}} API operations, grouped as in Alien's TypeScript SDK.
{{- if .Methods}}
// Operations without a group are its own methods, as in Alien's TypeScript SDK.
{{- end}}
type {{.Type}} struct {
	client *{{.Facade}}
{{range .Groups}}
	{{.Field}} *{{.Service}}
{{- end}}
}

func new{{.Type}}(client *{{.Facade}}) {{.Type}} {
	return {{.Type}}{
		client: client,
		{{- range .Groups}}
		{{.Field}}: &{{.Service}}{client},
		{{- end}}
	}
}

{{- range .Methods}}{{template "method" .}}{{end}}

{{- range .Groups}}

// {{.Service}} holds the {{$api.Name}} API's {{.Name}} operations.
type {{.Service}} struct{ client *{{$api.Facade}} }

{{- range .Methods}}{{template "method" .}}{{end}}
{{- end}}
{{- end}}

{{- define "method"}}

{{- range .Doc}}
// {{.}}
{{- end}}
func (s *{{.Receiver}}) {{.Name}}({{.Params}}) {{.Results}} {
	return {{.Wrap}}(baml_sdk.{{.Function}}({{.Args}}))
}

{{- with .Options}}

// {{.Type}} holds the optional arguments of {{.CallName}}; a nil *{{.Type}} sets none.
// Nil fields are left out of the request.
type {{.Type}} struct {
	{{- range .Fields}}
	{{- range .Doc}}
	// {{.}}
	{{- end}}
	{{.Name}} {{.Type}}
	{{- end}}
}

func (p *{{.Type}}) options() []baml_sdk.{{.Function}}Option {
	if p == nil {
		return nil
	}
	var options []baml_sdk.{{.Function}}Option
	{{- range .Fields}}
	if p.{{.Name}} != nil {
		options = append(options, baml_sdk.{{.Option}}({{.Arg}}))
	}
	{{- end}}
	return options
}
{{- end}}
{{- end}}
`))

var tableTemplate = template.Must(template.New(tableFile).Parse(`
// Code generated by internal/genops from baml_sdk/functions.go. DO NOT EDIT.

package alien

import (
	"context"
	"encoding/json"

	"example.com/alien-baml-sdk/go/baml_sdk"
)

// operations calls each generated operation through its method, by the
// operation's BAML path (conformance/README.md).
var operations = map[string]operation{
	{{- range .}}
	{{printf "%q" .Key}}: func(ctx context.Context, a map[string]json.RawMessage) (any, error) {
		{{- if .Result}}
		return result({{.Call}})
		{{- else}}
		return nil, {{.Call}}
		{{- end}}
	},
	{{- end}}
}
`))
