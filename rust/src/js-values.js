({
      project: value => {
      const paths = [], parents = new WeakMap(); let root = true;
      const text = JSON.stringify(value, function(key, field) {
        const first = root; root = false;
        if (field && typeof field === 'object') {
          const path = first ? [] : [...parents.get(this), key];
          const ancestor = parents.get(field);
          if (ancestor && ancestor.length < path.length && ancestor.every((part,index) => part === path[index])) {
            paths.push([path,path.length-ancestor.length]); return null;
          }
          parents.set(field, path);
        } else if (typeof field === 'number' && (!Number.isFinite(field) || Object.is(field, -0))) {
          paths.push([first ? [] : [...parents.get(this), key], Object.is(field, -0) ? '-0' : String(field)]);
        }
        return field;
      });
      return paths.length === 0 && typeof text === 'string' ? text : [text, JSON.stringify(paths)];
      },
      stringify: (value,space) => JSON.stringify(value,null,space),
      assign: (root,path,value,depth) => {
        if(depth){value=root;for(const key of path.slice(0,path.length-depth))value=value[key];}
        let target=root;
        for(let i=0;i<path.length-1;i++) target=target[path[i]];
        target[path[path.length-1]]=value;
      }
    })
