// expect: react/jsx-max-depth
export const Bad = () => (
  <div>
    <div>
      <div>
        <div>
          <div>
            <div>
              <div>
                <div />
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
);
